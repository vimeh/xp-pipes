//! Pure, bounded pipe growth. No graphics, clock or Wayland dependencies.
use crate::{
    config::{Joints, Pipes},
    materials::GOOD,
    rng::Random,
};

pub type Cell = [u16; 3];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Direction {
    pub axis: u8,
    pub sign: i16,
}
impl Direction {
    pub const ALL: [Self; 6] = [
        Self { axis: 0, sign: 1 },
        Self { axis: 0, sign: -1 },
        Self { axis: 1, sign: 1 },
        Self { axis: 1, sign: -1 },
        Self { axis: 2, sign: 1 },
        Self { axis: 2, sign: -1 },
    ];
    pub fn index(self) -> usize {
        usize::from(self.axis) * 2 + usize::from(self.sign < 0)
    }

    pub fn opposite(self) -> Self {
        Self {
            sign: -self.sign,
            ..self
        }
    }

    pub fn initial_notch(self) -> Self {
        Self::ALL[[2, 2, 5, 4, 2, 2][self.index()]]
    }

    fn cross(self, other: Self) -> Self {
        assert_ne!(self.axis, other.axis);
        let positive = (self.axis + 1) % 3 == other.axis;
        Self {
            axis: 3 - self.axis - other.axis,
            sign: self.sign * other.sign * if positive { 1 } else { -1 },
        }
    }

    /// Quarter-turn transport, equivalent to the reference's notch tables.
    pub fn turn(self, outgoing: Self, notch: Self) -> (u8, Self) {
        assert_ne!(self.axis, outgoing.axis);
        assert_ne!(self.axis, notch.axis);
        if notch == outgoing {
            (0, self.opposite())
        } else if notch == outgoing.opposite() {
            (2, self)
        } else if notch == outgoing.cross(self) {
            (1, notch)
        } else {
            (3, notch)
        }
    }

    /// Rotation about local +Z aligning local +Y with a world-space notch.
    pub fn roll_degrees(self, notch: Self) -> i16 {
        assert_ne!(self.axis, notch.axis);
        let y = self.initial_notch();
        let x = Self::ALL[[5, 4, 0, 0, 0, 1][self.index()]];
        if notch == y {
            0
        } else if notch == y.opposite() {
            180
        } else if notch == x {
            -90
        } else {
            90
        }
    }

    pub fn step(self, mut cell: Cell) -> Option<Cell> {
        let axis = usize::from(self.axis);
        cell[axis] = cell[axis].checked_add_signed(self.sign)?;
        Some(cell)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Segment {
        cell: Cell,
        direction: Direction,
        notch: Direction,
        long: bool,
        color: u8,
    },
    Joint {
        cell: Cell,
        color: u8,
    },
    Turn {
        cell: Cell,
        incoming: Direction,
        outgoing: Direction,
        variant: u8,
        color: u8,
    },
    Cap {
        cell: Cell,
        direction: Direction,
        notch: Direction,
        color: u8,
    },
    BallTurn {
        cell: Cell,
        incoming: Direction,
        outgoing: Direction,
        variant: u8,
        color: u8,
    },
    Reset,
}

/// Node storage need not match C++ layout, but traversal and reservations do.
struct Nodes {
    dims: Cell,
    occupied: Vec<u8>,
}

impl Nodes {
    fn new(dims: Cell) -> Self {
        assert!(dims.into_iter().all(|n| n > 0));
        Self {
            dims,
            occupied: vec![0; dims.into_iter().map(usize::from).product()],
        }
    }

    fn index(&self, [x, y, z]: Cell) -> usize {
        (usize::from(z) * usize::from(self.dims[1]) + usize::from(y)) * usize::from(self.dims[0])
            + usize::from(x)
    }

    fn free(&self, cell: Cell) -> bool {
        cell.iter().zip(self.dims).all(|(&v, n)| v < n) && self.occupied[self.index(cell)] == 0
    }

    fn claim(&mut self, cell: Cell) {
        debug_assert!(self.free(cell));
        let index = self.index(cell);
        self.occupied[index] = 1;
    }

    // 0100b39d: test before increment gives 6752 probes, not 6750/6751.
    fn find_start(&mut self, random: &mut Random) -> Option<Cell> {
        for _ in 0..6752 {
            let cell = self
                .dims
                .map(|n| u16::try_from(random.below(u32::from(n))).unwrap());
            if self.free(cell) {
                self.claim(cell);
                return Some(cell);
            }
        }
        // Ordered x/y/z scan, independent of the storage order. No fast full-grid
        // exit: even failed searches must consume the reference's random draws.
        for x in 0..self.dims[0] {
            for y in 0..self.dims[1] {
                for z in 0..self.dims[2] {
                    let cell = [x, y, z];
                    if self.free(cell) {
                        self.claim(cell);
                        return Some(cell);
                    }
                }
            }
        }
        None
    }

    // 0100b703: prefer free directions toward the leader, otherwise uniformly
    // choose any free neighbour. Unlike weight=100, even a sole exit draws RNG.
    fn preferred(
        &mut self,
        position: Cell,
        target: Cell,
        random: &mut Random,
    ) -> Option<Direction> {
        let mut choices = Direction::ALL;
        let mut count = 0;
        for direction in Direction::ALL {
            let axis = usize::from(direction.axis);
            let toward = if target[axis] > position[axis] { 1 } else { -1 };
            if target[axis] != position[axis]
                && direction.sign == toward
                && direction.step(position).is_some_and(|cell| self.free(cell))
            {
                choices[count] = direction;
                count += 1;
            }
        }
        if count == 0 {
            for direction in Direction::ALL {
                if direction.step(position).is_some_and(|cell| self.free(cell)) {
                    choices[count] = direction;
                    count += 1;
                }
            }
        }
        if count == 0 {
            return None;
        }
        let direction =
            choices[usize::try_from(random.below(u32::try_from(count).unwrap())).unwrap()];
        self.claim(direction.step(position).unwrap());
        Some(direction)
    }

    fn face_start(
        &mut self,
        face: usize,
        low: Cell,
        high: Cell,
        random: &mut Random,
    ) -> Option<Cell> {
        let axis = face / 2;
        // 0100b453 random/fallback order: X face -> Z,Y; Y -> X,Z; Z -> X,Y.
        let [first, second] = [[2, 1], [0, 2], [0, 1]][axis];
        let mut cell = [0; 3];
        cell[axis] = if face.is_multiple_of(2) {
            high[axis]
        } else {
            low[axis]
        };
        let area =
            u32::from(high[first] - low[first] + 1) * u32::from(high[second] - low[second] + 1);
        for _ in 0..=2 * area {
            cell[first] =
                u16::try_from(random.between(u32::from(low[first]), u32::from(high[first])))
                    .unwrap();
            cell[second] =
                u16::try_from(random.between(u32::from(low[second]), u32::from(high[second])))
                    .unwrap();
            if self.free(cell) {
                self.claim(cell);
                return Some(cell);
            }
        }
        for a in low[first]..=high[first] {
            for b in low[second]..=high[second] {
                cell[first] = a;
                cell[second] = b;
                if self.free(cell) {
                    self.claim(cell);
                    return Some(cell);
                }
            }
        }
        None
    }

    fn corner_start(&mut self, previous: Cell, random: &mut Random) -> Option<Cell> {
        let target = std::array::from_fn(|axis| {
            if previous[axis] < self.dims[axis] / 2 {
                self.dims[axis] - 1
            } else {
                0
            }
        });
        if self.free(target) {
            self.claim(target);
            return Some(target);
        }
        let (mut low, mut high) = (target, target);
        // The source caches max(X,Y)/3 on first use. Every supported XP viewport
        // has major dimension 15, so this process-global constant is always five.
        for _ in 0..5 {
            for axis in 0..3 {
                low[axis] = low[axis].saturating_sub(1);
                high[axis] = (high[axis] + 1).min(self.dims[axis] - 1);
            }
            let first = usize::try_from(random.below(6)).unwrap();
            for offset in 0..6 {
                if let Some(cell) = self.face_start((first + offset) % 6, low, high, random) {
                    return Some(cell);
                }
            }
        }
        self.find_start(random)
    }

    // 0100b67f: forward weight occupies the FIRST random buckets; turns remain
    // +X,-X,+Y,-Y,+Z,-Z ordered. A returned destination is already reserved.
    fn choose(
        &mut self,
        position: Cell,
        current: Direction,
        weight: u32,
        random: &mut Random,
    ) -> Option<Direction> {
        let forward = current.step(position).filter(|&cell| self.free(cell));
        let weight = if forward.is_some() { weight } else { 0 };
        if weight == 100 {
            self.claim(forward.unwrap());
            return Some(current);
        }
        let mut turns = Direction::ALL;
        let mut count = 0;
        for direction in Direction::ALL {
            if direction != current && direction.step(position).is_some_and(|cell| self.free(cell))
            {
                turns[count] = direction;
                count += 1;
            }
        }
        let total = u32::try_from(count).unwrap() + weight;
        if total == 0 {
            return None;
        }
        let pick = random.below(total);
        let direction = if pick < weight {
            current
        } else {
            turns[usize::try_from(pick - weight).unwrap()]
        };
        self.claim(direction.step(position).unwrap());
        Some(direction)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Growing,
    Trapped,
    NoStart,
    Retired,
}

struct Pipe {
    id: u8,
    textured: bool,
    following: bool,
    corner_restart: bool,
    position: Cell,
    direction: Direction,
    notch: Direction,
    material: u8,
    weight: u32,
    status: Status,
}

impl Pipe {
    fn new(random: &mut Random, id: u8, textured: bool) -> Self {
        let weight = if random.below(20) == 0 {
            random.between(25, 100)
        } else {
            random.below(4) + 1
        };
        Self {
            id,
            textured,
            following: false,
            corner_restart: false,
            position: [0; 3],
            direction: Direction::ALL[0],
            notch: Direction::ALL[2],
            material: 0,
            weight,
            status: Status::NoStart,
        }
    }

    fn choose(
        &self,
        nodes: &mut Nodes,
        random: &mut Random,
        leader: Option<Cell>,
    ) -> Option<Direction> {
        if let Some(target) = leader.filter(|&target| self.following && target != self.position) {
            nodes.preferred(self.position, target, random)
        } else {
            nodes.choose(self.position, self.direction, self.weight, random)
        }
    }

    fn start(
        &mut self,
        nodes: &mut Nodes,
        random: &mut Random,
        leader: Option<Cell>,
        events: &mut Vec<Event>,
    ) -> bool {
        let start = if self.corner_restart {
            nodes.corner_start(self.position, random)
        } else {
            nodes.find_start(random)
        };
        let Some(position) = start else {
            self.status = Status::NoStart;
            return false;
        };
        self.position = position;
        self.material = if self.textured {
            24 + u8::try_from(random.below(4)).unwrap()
        } else {
            GOOD[usize::try_from(random.below(16)).unwrap()]
        };
        self.direction = Direction::ALL[usize::try_from(random.below(6)).unwrap()];
        let Some(direction) = self.choose(nodes, random, leader) else {
            self.status = Status::Trapped;
            return true;
        };
        self.status = Status::Growing;
        self.notch = direction.initial_notch();
        self.cap(direction, events);
        self.segment(direction, false, events);
        true
    }

    fn cap(&self, direction: Direction, events: &mut Vec<Event>) {
        events.push(if self.textured {
            Event::Cap {
                cell: self.position,
                direction,
                notch: self.notch,
                color: self.material,
            }
        } else {
            Event::Joint {
                cell: self.position,
                color: self.material,
            }
        });
    }

    fn segment(&mut self, direction: Direction, long: bool, events: &mut Vec<Event>) {
        events.push(Event::Segment {
            cell: self.position,
            direction,
            notch: self.notch,
            long,
            color: self.material,
        });
        self.position = direction
            .step(self.position)
            .expect("selected destination is reserved");
        self.direction = direction;
    }

    fn draw(
        &mut self,
        nodes: &mut Nodes,
        random: &mut Random,
        leader: Option<Cell>,
        joint: u8,
        events: &mut Vec<Event>,
    ) {
        let Some(direction) = self.choose(nodes, random, leader) else {
            self.cap(self.direction, events);
            self.status = Status::Trapped;
            return;
        };
        let long = direction == self.direction;
        if !long {
            let (variant, notch) = self.direction.turn(direction, self.notch);
            if joint == 1 || (joint == 2 && random.below(3) == 0) {
                events.push(if self.textured {
                    Event::BallTurn {
                        cell: self.position,
                        incoming: self.direction,
                        outgoing: direction,
                        variant,
                        color: self.material,
                    }
                } else {
                    Event::Joint {
                        cell: self.position,
                        color: self.material,
                    }
                });
            } else {
                events.push(Event::Turn {
                    cell: self.position,
                    incoming: self.direction,
                    outgoing: direction,
                    variant,
                    color: self.material,
                });
            }
            self.notch = notch;
        }
        self.segment(direction, long, events);
    }
}

pub struct Simulation {
    nodes: Nodes,
    pipes: Vec<Pipe>,
    leader: Option<u8>,
    chase: bool,
    pipe_mode: Pipes,
    textured: bool,
    starts: u32,
    budget: u32,
    scene_length: u8,
    joints: Joints,
    current_joint: u8,
}
impl Simulation {
    pub fn new(
        dims: Cell,
        joints: Joints,
        pipe_mode: Pipes,
        textured: bool,
        scene_length: u8,
    ) -> Self {
        Self {
            nodes: Nodes::new(dims),
            pipes: Vec::with_capacity(4),
            leader: None,
            chase: false,
            pipe_mode,
            textured,
            starts: 0,
            budget: 0,
            scene_length,
            joints,
            current_joint: if joints == Joints::Cycle {
                2
            } else {
                joints as u8
            },
        }
    }

    pub fn ended(&self) -> bool {
        self.budget != 0 && self.pipes.is_empty()
    }

    fn leader_position(&self) -> Option<Cell> {
        self.pipes
            .iter()
            .find(|p| Some(p.id) == self.leader)
            .map(|p| p.position)
    }

    fn begin_scene(&mut self, random: &mut Random, events: &mut Vec<Event>) {
        if self.budget != 0 {
            events.push(Event::Reset);
        }
        self.nodes.occupied.fill(0);
        if self.joints == Joints::Cycle {
            self.current_joint = (self.current_joint + 1) % 3;
        }
        let base = if self.textured { 3 } else { 5 };
        let multiple = self.pipe_mode != Pipes::Fixed(1);
        self.budget = (if multiple { base * 3 / 2 } else { base }) * u32::from(self.scene_length);
        let count = match self.pipe_mode {
            Pipes::Fixed(count) => u32::from(count),
            Pipes::Multiple => {
                // The first random result is discarded by the reference's comparison.
                random.between(2, 4);
                random.between(2, 4)
            }
        };
        if multiple && random.below(5) == 0 {
            self.chase = true;
        }
        // Chase is deliberately sticky across natural scene resets (0100f5ce).
        self.leader = self.chase.then_some(0);
        self.starts = 0;
        for index in 0..count {
            let id = u8::try_from(index).unwrap();
            let mut pipe = Pipe::new(random, id, self.textured);
            // PickRandomTexture(1) still draws, once per new object, not restart.
            if self.textured {
                random.below(1);
            }
            pipe.following = self.chase && id != 0;
            let leader = self.leader_position();
            if !pipe.start(&mut self.nodes, random, leader, events) {
                pipe.status = Status::Retired;
            }
            pipe.corner_restart = self.chase && id == 0;
            self.pipes.push(pipe);
            self.starts += 1;
        }
    }

    /// One eligible `STATE::Render` call, including its startup/restart prepass.
    /// Scene fading is added by the presentation state machine in C09.
    pub fn tick(&mut self, random: &mut Random, events: &mut Vec<Event>) {
        events.clear();
        if self.pipes.is_empty() {
            self.begin_scene(random, events);
        }
        for index in 0..self.pipes.len() {
            let leader = self.leader_position();
            let pipe = &mut self.pipes[index];
            if pipe.status == Status::Trapped {
                self.starts += 1;
                if self.starts > self.budget {
                    pipe.status = Status::Retired;
                } else if !pipe.start(&mut self.nodes, random, leader, events) {
                    // A failed scan can cause undefined reads in XP. Retire safely
                    // and clamp future starts, preserving the completed probes.
                    self.budget = self.starts;
                    pipe.status = Status::Retired;
                }
            }
        }
        let leader_retired = self
            .pipes
            .iter()
            .any(|p| Some(p.id) == self.leader && p.status == Status::Retired);
        // Delay destruction until the prepass ends. Followers can still use the
        // retired leader's last valid position, never a freed C++ pointer.
        self.pipes.retain(|p| p.status != Status::Retired);
        if self.pipes.is_empty() {
            return;
        }
        if leader_retired {
            let index =
                usize::try_from(random.below(u32::try_from(self.pipes.len()).unwrap())).unwrap();
            let leader = &mut self.pipes[index];
            leader.following = false;
            leader.corner_restart = true;
            self.leader = Some(leader.id);
        }
        for index in 0..self.pipes.len() {
            let leader = self.leader_position();
            self.pipes[index].draw(&mut self.nodes, random, leader, self.current_joint, events);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn default_startup_matches_static_call_order_checkpoint() {
        // Nine post-warmup samples from the checked Start/Render call path:
        // constructor 20,4; position 15,8,15; material 16; heading 6; choices 9,4.
        let mut random = Random::new(Some(42));
        let mut simulation = Simulation::new([15, 8, 15], Joints::Elbow, Pipes::Fixed(1), false, 1);
        let mut events = Vec::new();
        simulation.tick(&mut random, &mut events);
        assert_eq!(random.checkpoint(), 0xb80f_4c37);
        let pipe = simulation.pipes.first().unwrap();
        assert_eq!(
            (pipe.weight, pipe.material, pipe.position, simulation.starts),
            (4, 16, [5, 0, 1], 1)
        );
        assert_eq!(
            events,
            [
                Event::Joint {
                    cell: [6, 1, 1],
                    color: 16
                },
                Event::Segment {
                    cell: [6, 1, 1],
                    direction: Direction::ALL[3],
                    notch: Direction::ALL[4],
                    long: false,
                    color: 16
                },
                Event::Turn {
                    cell: [6, 0, 1],
                    incoming: Direction::ALL[3],
                    outgoing: Direction::ALL[1],
                    variant: 1,
                    color: 16
                },
                Event::Segment {
                    cell: [6, 0, 1],
                    direction: Direction::ALL[1],
                    notch: Direction::ALL[4],
                    long: false,
                    color: 16
                },
            ]
        );
        assert_eq!(
            simulation
                .nodes
                .occupied
                .iter()
                .filter(|&&v| v != 0)
                .count(),
            3
        );
    }

    #[test]
    fn textured_startup_consumes_texture_selection_and_uses_textured_shapes() {
        let mut sim = Simulation::new([15, 8, 15], Joints::Elbow, Pipes::Fixed(1), true, 1);
        let mut random = Random::new(Some(42));
        let mut events = Vec::new();
        sim.tick(&mut random, &mut events);
        assert_eq!(random.checkpoint(), 0xccdc_561e);
        assert_eq!(sim.budget, 3);
        let pipe = &sim.pipes[0];
        assert_eq!((pipe.material, pipe.position), (26, [4, 1, 10]));
        assert!(matches!(
            events[0],
            Event::Cap {
                cell: [3, 0, 10],
                ..
            }
        ));
        let mut sim = Simulation::new([15, 8, 15], Joints::Ball, Pipes::Multiple, true, 1);
        sim.tick(&mut random, &mut events);
        assert_eq!(sim.budget, 4);
        for _ in 0..1000 {
            sim.tick(&mut random, &mut events);
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, Event::Joint { .. } | Event::Turn { .. }))
            );
            assert!(sim.pipes.iter().all(|p| (24..28).contains(&p.material)));
        }
    }

    #[test]
    fn fixed_pipe_counts_survive_scene_resets() {
        for count in 1..=4 {
            for seed in [0, 1, 42] {
                for textured in [false, true] {
                    let mut s = Simulation::new(
                        [15, 8, 15],
                        Joints::Elbow,
                        Pipes::Fixed(count),
                        textured,
                        1,
                    );
                    let mut random = Random::new(Some(seed));
                    let mut events = Vec::new();
                    let mut scenes = 0;
                    for _ in 0..10_000 {
                        let starting = s.pipes.is_empty();
                        s.tick(&mut random, &mut events);
                        if starting {
                            assert_eq!(s.pipes.len(), usize::from(count));
                            scenes += 1;
                            if scenes == 3 {
                                break;
                            }
                        }
                        assert!(s.pipes.len() <= usize::from(count));
                    }
                    assert_eq!(scenes, 3);
                }
            }
        }
    }

    #[test]
    fn longer_scenes_grow_more_before_reset_without_adding_heads() {
        let growth = [1, 2].map(|length| {
            let mut s = Simulation::new([15, 6, 15], Joints::Elbow, Pipes::Fixed(2), false, length);
            let mut random = Random::new(Some(42));
            let mut events = Vec::new();
            let mut segments = 0;
            for _ in 0..10_000 {
                s.tick(&mut random, &mut events);
                assert!(s.pipes.len() <= 2);
                segments += events
                    .iter()
                    .filter(|e| matches!(e, Event::Segment { .. }))
                    .count();
                if s.ended() {
                    break;
                }
            }
            assert!(s.ended(), "longer scenes must still finish");
            s.tick(&mut random, &mut events);
            assert_eq!(events.first(), Some(&Event::Reset));
            assert_eq!(s.budget, 7 * u32::from(length));
            assert_eq!(s.pipes.len(), 2);
            segments
        });
        assert!(
            growth[1] > growth[0],
            "double budget must allow more growth"
        );
    }

    #[test]
    fn multiple_startup_preserves_discarded_draw_and_actor_order() {
        let mut s = Simulation::new([15, 8, 15], Joints::Elbow, Pipes::Multiple, false, 1);
        let mut random = Random::new(Some(42));
        let mut events = Vec::new();
        s.tick(&mut random, &mut events);
        assert_eq!((s.starts, s.budget, s.chase), (4, 7, false));
        assert_eq!(random.checkpoint(), 0xdd7f_06b5);
        let actual: Vec<_> = s
            .pipes
            .iter()
            .map(|p| (p.id, p.weight, p.material, p.position, p.direction.index()))
            .collect();
        assert_eq!(
            actual,
            [
                (0, 1, 9, [11, 6, 6], 2),
                (1, 4, 4, [9, 2, 3], 1),
                (2, 4, 4, [1, 6, 6], 3),
                (3, 1, 19, [8, 7, 14], 4)
            ]
        );
        assert_eq!(s.nodes.occupied.iter().filter(|&&v| v != 0).count(), 12);
        let mut counts = [false; 3];
        for seed in 0..1000 {
            let mut s = Simulation::new([15, 8, 15], Joints::Elbow, Pipes::Multiple, false, 1);
            s.tick(&mut Random::new(Some(seed)), &mut events);
            counts[s.pipes.len() - 2] = true;
        }
        assert_eq!(counts, [true; 3]);
    }

    #[test]
    fn chase_survives_resets_and_replaces_a_retired_leader() {
        let mut s = Simulation::new([15, 8, 15], Joints::Mixed, Pipes::Multiple, false, 1);
        let mut random = Random::new(Some(0));
        let mut events = Vec::new();
        s.tick(&mut random, &mut events);
        assert!(s.chase);
        assert_eq!(s.leader, Some(0));
        assert!(s.pipes[0].corner_restart);
        s.starts = s.budget;
        s.pipes[0].status = Status::Trapped;
        s.tick(&mut random, &mut events);
        assert_eq!(s.pipes.iter().map(|p| p.id).collect::<Vec<_>>(), [1, 2, 3]);
        let leader = s.pipes.iter().find(|p| Some(p.id) == s.leader).unwrap();
        assert!(!leader.following && leader.corner_restart);
        assert_eq!(s.pipes.iter().filter(|p| !p.following).count(), 1);
        let mut resets = 0;
        for _ in 0..4000 {
            s.tick(&mut random, &mut events);
            assert!(s.chase, "the reference never clears chase on normal reset");
            if events.first() == Some(&Event::Reset) {
                resets += 1;
            }
        }
        assert!(resets > 2);
    }

    #[test]
    fn preferred_exits_and_corner_search_preserve_draws_and_reservations() {
        let mut n = Nodes::new([15; 3]);
        let mut random = Random::new(Some(42));
        let checkpoint = random.checkpoint();
        assert_eq!(n.corner_start([0; 3], &mut random), Some([14; 3]));
        assert_eq!(
            random.checkpoint(),
            checkpoint,
            "free opposite corner draws nothing"
        );
        assert_eq!(n.corner_start([14; 3], &mut random), Some([0; 3]));
        n.occupied.fill(1);
        let next = n.index([8, 7, 7]);
        n.occupied[next] = 0;
        let mut expected = random.clone();
        expected.below(1);
        assert_eq!(
            n.preferred([7; 3], [14, 7, 7], &mut random),
            Some(Direction::ALL[0])
        );
        assert_eq!(random.checkpoint(), expected.checkpoint());
        let fallback = n.index([7, 6, 7]);
        n.occupied[fallback] = 0;
        assert_eq!(
            n.preferred([7; 3], [14, 7, 7], &mut random),
            Some(Direction::ALL[3])
        );
        n.occupied.fill(1);
        expected = random.clone();
        assert_eq!(n.face_start(0, [2, 3, 4], [2, 3, 4], &mut random), None);
        assert_eq!(
            random.checkpoint(),
            expected.checkpoint(),
            "equal face bounds consume no random values"
        );
        assert_eq!(n.face_start(4, [0, 0, 0], [1, 1, 0], &mut random), None);
        for _ in 0..18 {
            expected.below(2);
        } // 2*area+1 attempts, two draws each
        assert_eq!(random.checkpoint(), expected.checkpoint());
        let near = n.index([13, 14, 14]);
        n.occupied[near] = 0;
        assert_eq!(n.corner_start([0; 3], &mut random), Some([13, 14, 14]));
        assert!(!n.free([13, 14, 14]));
    }

    #[test]
    fn joint_modes_have_original_selection_and_random_consumption() {
        for (mode, checkpoint, ball) in [
            (Joints::Elbow, 0xb80f_4c37, false),
            (Joints::Ball, 0xb80f_4c37, true),
            (Joints::Mixed, 0xccdc_561e, false),
            (Joints::Cycle, 0xb80f_4c37, false),
        ] {
            let mut random = Random::new(Some(42));
            let mut sim = Simulation::new([15, 8, 15], mode, Pipes::Fixed(1), false, 1);
            let mut events = Vec::new();
            sim.tick(&mut random, &mut events);
            assert_eq!(random.checkpoint(), checkpoint);
            assert_eq!(matches!(events[2], Event::Joint { .. }), ball);
            assert_eq!(sim.pipes[0].notch, Direction::ALL[4]);
        }
        let mut sim = Simulation::new([15, 8, 15], Joints::Cycle, Pipes::Fixed(1), false, 1);
        let mut random = Random::new(Some(1));
        let mut events = Vec::new();
        let mut sequence = Vec::new();
        for frame in 0..20_000 {
            sim.tick(&mut random, &mut events);
            if frame == 0 || events.first() == Some(&Event::Reset) {
                sequence.push(sim.current_joint);
                if sequence.len() == 7 {
                    break;
                }
            }
        }
        assert_eq!(sequence, [0, 1, 2, 0, 1, 2, 0]);
    }

    #[test]
    fn construction_weight_survives_restarts_and_no_length_cap_exists() {
        let mut random = Random::new(Some(1));
        let mut pipe = Pipe::new(&mut random, 0, false);
        let weight = pipe.weight;
        let mut events = Vec::new();
        for _ in 0..20 {
            let mut nodes = Nodes::new([15; 3]);
            assert!(pipe.start(&mut nodes, &mut random, None, &mut events));
            assert_eq!(pipe.weight, weight);
        }
        // A forced straight corridor exercises >100 segments without a run cap.
        let mut nodes = Nodes::new([1, 1, 250]);
        nodes.claim([0, 0, 0]);
        pipe.position = [0; 3];
        pipe.direction = Direction::ALL[4];
        pipe.weight = 100;
        pipe.notch = Direction::ALL[2];
        pipe.status = Status::Growing;
        let checkpoint = random.checkpoint();
        for _ in 0..200 {
            pipe.draw(&mut nodes, &mut random, None, 2, &mut events);
        }
        assert_eq!(
            random.checkpoint(),
            checkpoint,
            "mixed selection only runs on turns"
        );
        assert_eq!(pipe.position, [0, 0, 200]);
        assert_eq!(pipe.status, Status::Growing);
    }

    #[test]
    fn deterministic_bounded_nonintersecting_and_resetting() {
        for (dims, pipes) in [
            ([2, 2, 2], Pipes::Fixed(1)),
            ([15, 8, 15], Pipes::Fixed(1)),
            ([1, 15, 15], Pipes::Fixed(1)),
            ([15, 8, 15], Pipes::Fixed(2)),
            ([1, 15, 15], Pipes::Fixed(2)),
            ([15, 8, 15], Pipes::Multiple),
            ([1, 15, 15], Pipes::Multiple),
        ] {
            let multiple = pipes != Pipes::Fixed(1);
            let (mut a, mut b) = (
                Simulation::new(dims, Joints::Elbow, pipes, false, 1),
                Simulation::new(dims, Joints::Elbow, pipes, false, 1),
            );
            let (mut ea, mut eb) = (Vec::new(), Vec::new());
            let mut ra = Random::new(Some(1234));
            let mut rb = ra.clone();
            let mut occupied = HashSet::new();
            let mut resets = 0;
            for _ in 0..3000 {
                a.tick(&mut ra, &mut ea);
                b.tick(&mut rb, &mut eb);
                assert_eq!(ea, eb);
                assert_eq!(ra.checkpoint(), rb.checkpoint());
                assert!(ea.len() <= if multiple { 17 } else { 5 });
                assert!(a.starts <= if multiple { 11 } else { 6 });
                for event in &ea {
                    match *event {
                        Event::Reset => {
                            occupied.clear();
                            resets += 1;
                        }
                        Event::Segment {
                            cell, direction, ..
                        } => {
                            assert!(occupied.contains(&cell));
                            let target = direction.step(cell).unwrap();
                            assert!(target.iter().zip(dims).all(|(&v, n)| v < n));
                            assert!(occupied.insert(target), "intersection at {target:?}");
                        }
                        Event::Joint { cell, .. }
                        | Event::Turn { cell, .. }
                        | Event::Cap { cell, .. }
                        | Event::BallTurn { cell, .. } => {
                            occupied.insert(cell);
                        }
                    }
                }
            }
            assert!(resets > 1);
        }
    }

    #[test]
    fn trapped_pipe_restarts_before_drawing_and_retires_at_budget() {
        let mut s = Simulation::new([15; 3], Joints::Elbow, Pipes::Fixed(1), false, 1);
        let mut random = Random::new(Some(1));
        let mut events = Vec::new();
        s.tick(&mut random, &mut events);
        let weight = s.pipes[0].weight;
        for starts in 2..=5 {
            s.pipes[0].status = Status::Trapped;
            s.tick(&mut random, &mut events);
            assert_eq!(s.starts, starts);
            assert_eq!(s.pipes[0].weight, weight);
            assert!(matches!(events[0], Event::Joint { .. }));
            assert!(!events.contains(&Event::Reset));
        }
        s.pipes[0].status = Status::Trapped;
        s.tick(&mut random, &mut events);
        assert!(s.pipes.is_empty());
        assert_eq!(events, Vec::<Event>::new());
        s.tick(&mut random, &mut events);
        assert_eq!(events[0], Event::Reset);
        assert_eq!(s.starts, 1);
        s.nodes.occupied.fill(1);
        s.pipes[0].status = Status::Trapped;
        s.tick(&mut random, &mut events);
        assert!(s.pipes.is_empty()); // No undefined access after an exhausted scan.
    }

    #[test]
    fn xp_node_reservations_and_rng_consumption() {
        let mut random = Random::new(Some(42));
        let mut expected = random.clone();
        let mut nodes = Nodes::new([3, 3, 3]);
        nodes.occupied.fill(1);
        let origin = [1, 1, 1];
        let ahead = nodes.index([2, 1, 1]);
        nodes.occupied[ahead] = 0;
        assert_eq!(
            nodes.choose(origin, Direction::ALL[0], 0, &mut random),
            None
        );
        assert!(nodes.free([2, 1, 1]));
        assert_eq!(
            nodes.choose(origin, Direction::ALL[0], 100, &mut random),
            Some(Direction::ALL[0])
        );
        assert!(!nodes.free([2, 1, 1]));
        assert_eq!(random.below(32768), expected.below(32768));
        let turn = nodes.index([1, 2, 1]);
        nodes.occupied[turn] = 0;
        assert_eq!(
            nodes.choose(origin, Direction::ALL[0], 100, &mut random),
            Some(Direction::ALL[2])
        );
        expected.below(1);
        assert_eq!(random.below(32768), expected.below(32768));
        assert_eq!(
            nodes.choose(origin, Direction::ALL[0], 1, &mut random),
            None
        );

        let mut one = Nodes::new([1, 1, 1]);
        assert_eq!(one.find_start(&mut random), Some([0, 0, 0]));
        assert!(!one.free([0, 0, 0]));
        expected = random.clone();
        assert_eq!(one.find_start(&mut random), None);
        for _ in 0..6752 * 3 {
            expected.below(1);
        }
        assert_eq!(random.below(32768), expected.below(32768));
    }

    #[test]
    fn xp_fallback_scans_xyz_not_flat_storage_order() {
        // With n=40000, the 15-bit scaled sampler can never return z=5.
        // This forces the actual probe-limit path without a fake RNG.
        assert!((0_u64..32768).all(|r| r * 40000 / 32768 != 5));
        let mut nodes = Nodes::new([2, 2, 40000]);
        nodes.occupied.fill(1);
        for cell in [[0, 1, 5], [1, 0, 5]] {
            let index = nodes.index(cell);
            nodes.occupied[index] = 0;
        }
        let mut random = Random::new(Some(1));
        assert_eq!(nodes.find_start(&mut random), Some([0, 1, 5]));
        assert_eq!(nodes.find_start(&mut random), Some([1, 0, 5]));
        assert_eq!(nodes.find_start(&mut random), None);
    }

    #[test]
    fn orientation_rules_match_all_valid_pe_table_entries() {
        let fixture = include_str!("../tests/fixtures/xp-orientation-v1.tsv");
        assert!(fixture.starts_with("schema\txp-orientation-v1\nsha256\t664b4b1da036eb36b13a3bce2fd572e37127223cbf6796adde12cbfb3af0d893\nevidence\tstatic-data\n"));
        let mut count = 0;
        for line in fixture.lines().skip(4) {
            let v: Vec<i16> = line.split('\t').map(|x| x.parse().unwrap()).collect();
            assert_eq!(v.len(), 7);
            let [a, b, n, next] =
                [v[0], v[1], v[2], v[4]].map(|i| Direction::ALL[usize::try_from(i).unwrap()]);
            assert_eq!(a.turn(b, n), (u8::try_from(v[3]).unwrap(), next));
            assert_eq!(b.roll_degrees(a.opposite()), v[5]);
            assert_eq!(a.roll_degrees(n), v[6]);
            count += 1;
        }
        assert_eq!(count, 96);
    }
}

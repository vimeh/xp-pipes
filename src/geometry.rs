//! XP normal-pipe meshes and transforms, reconstructed from checked instructions.
use crate::simulation::{Cell, Direction, Event};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};
use std::f32::consts::{FRAC_PI_2, PI, SQRT_2, TAU};

pub const SPACING: f32 = 7.0;
pub const SLICES: [u16; 5] = [8, 12, 20, 28, 36];
const ROTATION_STEP: f32 = f32::from_bits(0x411b_b478);

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Instance {
    pub center: [f32; 3],
    pub rotation: [f32; 4],
    pub material: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Short,
    Long,
    Ball,
    Elbow0,
    Elbow1,
    Elbow2,
    Elbow3,
    TexturedCap,
    TexturedBall0,
    TexturedBall1,
    TexturedBall2,
    TexturedBall3,
}
impl Primitive {
    fn textured_ball(variant: u8) -> Self {
        [
            Self::TexturedBall0,
            Self::TexturedBall1,
            Self::TexturedBall2,
            Self::TexturedBall3,
        ][usize::from(variant)]
    }

    fn elbow(variant: u8) -> Self {
        [Self::Elbow0, Self::Elbow1, Self::Elbow2, Self::Elbow3][usize::from(variant)]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub primitive: Primitive,
    pub start: usize,
    pub count: usize,
}

#[derive(Default)]
pub struct Scene {
    pub instances: Vec<Instance>,
    pub spans: Vec<Span>,
    pub generation: u64,
    pub fades: u32,
    pub angle: f32,
    next_angle: f32,
}
impl Scene {
    /// Layout is bounded to 15 nodes/axis. Allow commands/caps for every exposed
    /// mode (at most seven starts), doubled Vec/GPU capacities and span records.
    pub fn model_bytes(dims: Cell) -> u64 {
        let cells = dims.into_iter().map(u64::from).product::<u64>();
        let per_instance =
            u64::try_from(4 * size_of::<Instance>() + 2 * size_of::<Span>()).unwrap();
        (3 * cells + 80) * per_instance + cells
    }

    pub fn retained_bytes(&self) -> usize {
        self.instances.capacity() * size_of::<Instance>()
            + self.spans.capacity() * size_of::<Span>()
    }

    pub fn reset(&mut self, completed: bool) {
        self.instances.clear();
        self.spans.clear();
        self.fades = 0;
        self.generation = self.generation.wrapping_add(1);
        // FrameReset installs the old angle in the matrix stack before updating
        // VIEW's angle for a later scene. The first two natural scenes use zero.
        self.angle = self.next_angle;
        if completed {
            self.next_angle += ROTATION_STEP;
            if self.next_angle >= 360.0 {
                self.next_angle -= 360.0;
            }
        }
    }

    pub fn push(&mut self, primitive: Primitive, instance: Instance) {
        if let Some(span) = self.spans.last_mut().filter(|s| s.primitive == primitive) {
            span.count += 1;
        } else {
            self.spans.push(Span {
                primitive,
                start: self.instances.len(),
                count: 1,
            });
        }
        self.instances.push(instance);
    }

    pub fn apply(&mut self, events: &[Event], dims: Cell) {
        for &event in events {
            let (cell, material, primitive, rotation, translation) = match event {
                Event::Reset => {
                    self.reset(true);
                    continue;
                }
                Event::Joint { cell, color } => {
                    (cell, color, Primitive::Ball, Quat::IDENTITY, Vec3::ZERO)
                }
                Event::Cap {
                    cell,
                    direction,
                    notch,
                    color,
                } => (
                    cell,
                    color,
                    Primitive::TexturedCap,
                    align_notch(direction, notch),
                    Vec3::ZERO,
                ),
                Event::Segment {
                    cell,
                    direction,
                    notch,
                    long,
                    color,
                } => {
                    let rotation = align_notch(direction, notch);
                    (
                        cell,
                        color,
                        if long {
                            Primitive::Long
                        } else {
                            Primitive::Short
                        },
                        rotation,
                        vector(direction) * if long { -1.0 } else { 1.0 },
                    )
                }
                Event::Turn {
                    cell,
                    incoming,
                    outgoing,
                    variant,
                    color,
                }
                | Event::BallTurn {
                    cell,
                    incoming,
                    outgoing,
                    variant,
                    color,
                } => {
                    let rotation =
                        align_z(outgoing) * roll(outgoing.roll_degrees(incoming.opposite()));
                    (
                        cell,
                        color,
                        if matches!(event, Event::BallTurn { .. }) {
                            Primitive::textured_ball(variant)
                        } else {
                            Primitive::elbow(variant)
                        },
                        rotation,
                        vector(outgoing),
                    )
                }
            };
            let center = Vec3::from_array(std::array::from_fn(|i| {
                (f32::from(cell[i]) - (f32::from(dims[i]) - 1.0) * 0.5) * SPACING
            }));
            self.push(
                primitive,
                Instance {
                    center: (center + translation).to_array(),
                    rotation: rotation.to_array(),
                    material: u32::from(material),
                },
            );
        }
    }
}

fn vector(direction: Direction) -> Vec3 {
    let mut v = Vec3::ZERO;
    v[usize::from(direction.axis)] = f32::from(direction.sign);
    v
}

fn align_notch(direction: Direction, notch: Direction) -> Quat {
    let rotation = if direction.axis == notch.axis {
        // The original table's parallel-axis sentinel is -0.01 degrees. A
        // trapped restart can retain such a notch; do not index invalid memory.
        Quat::from_rotation_z(single(
            f64::from(-0.01_f32) * f64::from(PI) * f64::from(f32::from_bits(0x3bb6_0b61)),
        ))
    } else {
        roll(direction.roll_degrees(notch))
    };
    align_z(direction) * rotation
}

fn align_z(direction: Direction) -> Quat {
    match direction.index() {
        0 => Quat::from_rotation_y(FRAC_PI_2),
        1 => Quat::from_rotation_y(-FRAC_PI_2),
        2 => Quat::from_rotation_x(-FRAC_PI_2),
        3 => Quat::from_rotation_x(FRAC_PI_2),
        4 => Quat::IDENTITY,
        5 => Quat::from_rotation_y(PI),
        _ => unreachable!(),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "explicit single-precision store after extended arithmetic"
)]
fn single(value: f64) -> f32 {
    value as f32
}

fn roll(degrees: i16) -> Quat {
    Quat::from_rotation_z(single(
        f64::from(degrees) * f64::from(PI) * f64::from(f32::from_bits(0x3bb6_0b61)),
    ))
}

/// VIEW uses a stored f32 aspect, then x87 arithmetic and truncation.
pub fn layout(size: [u32; 2]) -> Result<Cell, String> {
    let [width, height] = size.map(u16::try_from);
    let width = width.map_err(|_| "viewport width exceeds supported logical range")?;
    let height = height.map_err(|_| "viewport height exceeds supported logical range")?;
    if width == 0 || height == 0 {
        return Err("zero viewport dimension".into());
    }
    let aspect = f64::from(f32::from(width) / f32::from(height));
    let short = if width >= height {
        15.0 / aspect
    } else {
        15.0 * aspect
    };
    let short = (1..=15_u16)
        .rev()
        .find(|&n| f64::from(n) <= short)
        .unwrap_or(1);
    Ok(if width >= height {
        [15, short, 15]
    } else {
        [short, 15, 15]
    })
}

/// Keep XP's LH transforms. Remap clip z from D3D's [0,w] to GL's [-w,w].
pub fn camera(aspect: f32, angle: f32) -> (Mat4, Mat4) {
    let view = Mat4::from_cols_array(&[
        -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 75.0, 1.0,
    ]) * Mat4::from_rotation_y(angle);
    let (near, far) = (1.0, 299.0);
    let f = 1.0 / (FRAC_PI_2 * 0.5).tan();
    let projection = Mat4::from_cols_array(&[
        f / aspect,
        0.0,
        0.0,
        0.0,
        0.0,
        f,
        0.0,
        0.0,
        0.0,
        0.0,
        (far + near) / (far - near),
        1.0,
        0.0,
        0.0,
        -2.0 * near * far / (far - near),
        0.0,
    ]);
    (view, projection)
}

#[derive(Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
}

/// Exact vertex/index storage for the bounded mesh recipes, without allocating
/// those meshes every time the event loop checks its aggregate resource budget.
pub fn mesh_bytes(tessellation: u8, textured: bool) -> u64 {
    let slices = u64::from(SLICES[usize::from(tessellation)]);
    let short = (5 * slices).div_ceil(7);
    if textured {
        let indices = 6 * slices * short + 54 * slices * slices;
        indices * u64::try_from(size_of::<Vertex>() + size_of::<u16>()).unwrap()
    } else {
        let indices = 6 * slices * short + 24 * slices * slices;
        let vertices = (short + 1) * (slices + 1)
            + 2 * (slices + 1).pow(2)
            + 4 * (slices / 2 + 1) * (slices + 1);
        vertices * u64::try_from(size_of::<Vertex>()).unwrap() + indices * 2
    }
}

pub fn meshes(tessellation: u8, repeats: Option<[u16; 2]>) -> Vec<Mesh> {
    let slices = SLICES[usize::from(tessellation)];
    let radius = single(f64::from(SQRT_2) / (f64::from(PI) / f64::from(slices)).cos());
    let bend = repeats.map(|r| [0.0, single(2.0 * f64::from(r[1]) / 7.0), f32::from(r[1])]);
    let short = bend.map(|[_, end, v]| [end, v, v]);
    let long = repeats.map(|r| [0.0, f32::from(r[1]), f32::from(r[1])]);
    let mut result = vec![
        tube(slices, 5, short),
        tube(slices, 7, long),
        sphere(slices, radius, None),
        elbow(slices, 0, bend),
        elbow(slices, 1, bend),
        elbow(slices, 2, bend),
        elbow(slices, 3, bend),
    ];
    if let Some(repeats) = repeats {
        let factor = f64::from(repeats[0]) * f64::from(1.0_f32 / SPACING);
        let low = -factor * f64::from(f32::from_bits(0x3ed4_13cc));
        let offset = 1.0 - low.trunc();
        let start = single(offset + f64::from(single(low)));
        let end = single(offset + factor * f64::from(f32::from_bits(0x401a_827a)));
        // Sphere UVs run from param_4 at +Z to param_3 at -Z.
        result.push(sphere(
            slices,
            SQRT_2,
            Some([end, start, f32::from(repeats[1])]),
        ));
        for variant in 0..4 {
            result.push(ball_joint(slices, variant, bend));
        }
        for mesh in &mut result {
            wrap_coordinates(mesh);
        }
    }
    result
}

fn vertex(position: Vec3, normal: Vec3, uv: [f32; 2]) -> Vertex {
    Vertex {
        position: position.to_array(),
        normal: normal.to_array(),
        uv,
    }
}

fn uv(row: u16, col: u16, rows: u16, columns: u16, bounds: Option<[f32; 3]>) -> [f32; 2] {
    bounds.map_or([0.0; 2], |[start, end, repeat]| {
        [
            single(f64::from(row) * f64::from(end - start) / f64::from(rows) + f64::from(start)),
            single(f64::from(col) * f64::from(repeat) / f64::from(columns)),
        ]
    })
}

// Triangle-list order from the source's interleaved strip conversion. These
// indices need not have the conventional RH outward winding; C08 sets culling
// for the preserved LH pipeline, rather than rewriting topology to hide it.
fn connect(mesh: &mut Mesh, rows: u16, columns: u16, sphere: bool) {
    let width = columns + 1;
    for row in 0..rows {
        for col in 0..columns {
            let (mut a, mut b) = (row * width + col, (row + 1) * width + col);
            if sphere {
                std::mem::swap(&mut a, &mut b);
            }
            mesh.indices.extend([b, a, b + 1, a, a + 1, b + 1]);
        }
    }
}

fn tube(slices: u16, length: u16, bounds: Option<[f32; 3]>) -> Mesh {
    // 0100ca6e: ceil(slices * length / node spacing), capped at 99.
    let stacks = (slices * length).div_ceil(7).min(99);
    let mut mesh = Mesh::default();
    for row in 0..=stacks {
        for col in 0..=slices {
            let theta = f64::from(TAU) * f64::from(col % slices) / f64::from(slices);
            // Source rounds the angle for FCOS, but not for FSIN.
            let (sin, cos) = (single(theta.sin()), single(f64::from(single(theta)).cos()));
            let normal = Vec3::new(sin, cos, 0.0);
            mesh.vertices.push(vertex(
                Vec3::new(
                    sin,
                    cos,
                    f32::from(row) * f32::from(length) / f32::from(stacks),
                ),
                normal,
                uv(row, col, stacks, slices, bounds),
            ));
        }
    }
    connect(&mut mesh, stacks, slices, false);
    mesh
}

fn sphere(slices: u16, radius: f32, bounds: Option<[f32; 3]>) -> Mesh {
    let mut mesh = Mesh::default();
    for row in 0..=slices {
        let (radial, z) = (f64::from(PI) * f64::from(row) / f64::from(slices)).sin_cos();
        for col in 0..=slices {
            let theta = f64::from(TAU) * f64::from(col % slices) / f64::from(slices);
            let (x, y) = (single(theta.sin()), single(f64::from(single(theta)).cos()));
            let normal = Vec3::new(single(radial) * x, single(radial) * y, single(z));
            let r = if row == 0 || row == slices {
                0.0
            } else {
                single(radial * f64::from(radius))
            };
            mesh.vertices.push(vertex(
                Vec3::new(r * x, r * y, single(z) * radius),
                normal,
                uv(row, col, slices, slices, bounds),
            ));
        }
    }
    connect(&mut mesh, slices, slices, true);
    mesh
}

fn elbow(slices: u16, variant: u16, bounds: Option<[f32; 3]>) -> Mesh {
    // 0100d3da + 0100c3a6: rotate rings about (0,r,0), NOT the origin.
    let stacks = slices / 2;
    let pivot = Vec3::Y;
    let mut mesh = Mesh::default();
    for row in 0..=stacks {
        let rotation = Quat::from_rotation_x(-FRAC_PI_2 * f32::from(row) / f32::from(stacks));
        let center = pivot + rotation * -Vec3::Z;
        for col in 0..=slices {
            let theta = f64::from(TAU) * f64::from(col) / f64::from(slices)
                + f64::from(variant) * f64::from(FRAC_PI_2);
            let (sin, cos) = theta.sin_cos();
            let position = pivot + rotation * Vec3::new(single(sin), 0.0, single(cos - 1.0));
            mesh.vertices.push(vertex(
                position,
                (position - center).normalize(),
                uv(row, col, stacks, slices, bounds),
            ));
        }
    }
    connect(&mut mesh, stacks, slices, false);
    mesh
}

fn ball_joint(slices: u16, variant: u16, bounds: Option<[f32; 3]>) -> Mesh {
    let mut mesh = Mesh::default();
    let step = TAU / f32::from(slices);
    for row in 0..=slices {
        let sweep = f64::from(FRAC_PI_2) * f64::from(row) / f64::from(slices);
        let radius =
            single((f64::from(std::f32::consts::FRAC_PI_4) + sweep).sin() * f64::from(SQRT_2));
        let rotation = Quat::from_rotation_x(single(-sweep));
        let mut angle = f64::from(f32::from(variant) * FRAC_PI_2);
        for col in 0..=slices {
            let position = Vec3::Y
                + rotation
                    * Vec3::new(
                        single(angle.sin() * f64::from(radius)),
                        0.0,
                        single(angle.cos() * f64::from(radius) - f64::from(radius)),
                    );
            // Source normals use fixed centre (0,0,-r), unlike the elbow's ring centre.
            mesh.vertices.push(vertex(
                position,
                (position + Vec3::Z).normalize(),
                uv(row, col, slices, slices, bounds),
            ));
            angle += f64::from(step);
        }
    }
    connect(&mut mesh, slices, slices, false);
    mesh
}

fn wrap_coordinates(mesh: &mut Mesh) {
    // D3DRS_WRAP0 is pre-interpolation wrapping, distinct from sampler REPEAT.
    // ponytail: XP supplies out-of-range UVs (undefined by the API); extend the
    // documented shortest-path rule periodically until a pinned driver trace
    // establishes its particular behavior. Preserve half-period ties.
    let mut vertices = Vec::with_capacity(mesh.indices.len());
    for triangle in mesh.indices.as_chunks::<3>().0 {
        let mut face = triangle.map(|i| mesh.vertices[usize::from(i)]);
        for component in 0..2 {
            let origin = face[0].uv[component];
            for vertex in &mut face[1..] {
                let delta = vertex.uv[component] - origin;
                if delta > 0.5 {
                    vertex.uv[component] -= (delta - 0.5).ceil();
                } else if delta < -0.5 {
                    vertex.uv[component] -= (delta + 0.5).floor();
                }
            }
        }
        vertices.extend(face);
    }
    mesh.indices = (0..vertices.len())
        .map(|i| u16::try_from(i).expect("bounded tessellation"))
        .collect();
    mesh.vertices = vertices;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_layout_projection_and_scene_angles() {
        for (size, expected) in [
            ([640, 480], [15, 11, 15]),
            ([1920, 1080], [15, 8, 15]),
            ([5120, 2160], [15, 6, 15]),
            ([2160, 3840], [8, 15, 15]),
            ([1000, 1000], [15; 3]),
            ([16384, 1], [15, 1, 15]),
        ] {
            assert_eq!(layout(size).unwrap(), expected);
        }
        assert_eq!(layout([2560, 1080]), layout([5120, 2160]));
        assert!(layout([0, 1]).is_err());
        assert!(layout([u32::MAX, 1]).is_err());
        let (view, projection) = camera(4.0 / 3.0, 0.0);
        assert_eq!(view.transform_point3(Vec3::ZERO), Vec3::new(0.0, 0.0, 75.0));
        assert!((projection.project_point3(Vec3::Z).z + 1.0).abs() < 1e-6);
        assert!((projection.project_point3(Vec3::Z * 299.0).z - 1.0).abs() < 1e-6);
        assert!((projection.y_axis.y / projection.x_axis.x - 4.0 / 3.0).abs() < 1e-6);
        let mut scene = Scene::default();
        scene.reset(true);
        assert_eq!(scene.angle.to_bits(), 0);
        scene.reset(true);
        assert_eq!(scene.angle.to_bits(), ROTATION_STEP.to_bits());
        scene.reset(false);
        assert_eq!(scene.angle.to_bits(), (ROTATION_STEP * 2.0).to_bits());
        for _ in 0..100 {
            scene.reset(true);
            assert!((0.0..360.0).contains(&scene.angle));
        }
    }

    #[test]
    fn preserved_lh_sphere_front_projects_counterclockwise_in_gl() {
        let sphere = &meshes(0, None)[2];
        let (view, projection) = camera(1.0, 0.0);
        let points: Vec<_> = sphere.indices[3..6]
            .iter()
            .map(|&index| {
                (projection * view)
                    .project_point3(Vec3::from_array(
                        sphere.vertices[usize::from(index)].position,
                    ))
                    .truncate()
            })
            .collect();
        assert!((points[1] - points[0]).perp_dot(points[2] - points[0]) > 0.0);
    }

    #[test]
    fn reference_portrait_camera_can_clip_a_complete_initial_cap() {
        let mut scene = Scene::default();
        scene.apply(
            &[Event::Joint {
                cell: [0, 14, 14],
                color: 0,
            }],
            [6, 15, 15],
        );
        let (view, projection) = camera(540.0 / 1280.0, 0.0);
        let center = Vec3::from_array(scene.instances[0].center);
        assert!(meshes(0, None)[2].vertices.iter().all(|v| {
            let clip = projection * view * (Vec3::from_array(v.position) + center).extend(1.0);
            clip.x > clip.w
        }));
    }

    #[test]
    fn mesh_dimensions_topology_and_normals() {
        assert_eq!(size_of::<Vertex>(), 32);
        assert_eq!(size_of::<Instance>(), 32);
        assert_eq!(std::mem::offset_of!(Instance, rotation), 12);
        assert_eq!(std::mem::offset_of!(Instance, material), 28);
        for level in 0..5 {
            let s = SLICES[usize::from(level)];
            let all = meshes(level, None);
            for mesh in &all {
                assert!(
                    mesh.indices
                        .iter()
                        .all(|&i| usize::from(i) < mesh.vertices.len())
                );
                for v in &mesh.vertices {
                    assert!(v.position.iter().all(|n| n.is_finite()));
                    assert!((Vec3::from_array(v.normal).length() - 1.0).abs() < 1e-5);
                }
            }
            assert_eq!(
                all[0].indices.len(),
                usize::from((s * 5).div_ceil(7) * s) * 6
            );
            assert_eq!(all[1].indices.len(), usize::from(s * s) * 6);
            assert_eq!(all[2].indices.len(), usize::from(s * s) * 6);
            assert_eq!(all[3].indices.len(), usize::from(s / 2 * s) * 6);
            assert_eq!(&all[0].indices[..6], &[s + 1, 0, s + 2, 0, 1, s + 2]);
            assert_eq!(
                all[0].vertices[0].position.map(f32::to_bits),
                [0.0_f32, 1.0, 0.0].map(f32::to_bits)
            );
            assert_eq!(
                all[0].vertices.last().unwrap().position[2].to_bits(),
                5.0_f32.to_bits()
            );
            assert_eq!(
                all[1].vertices.last().unwrap().position[2].to_bits(),
                7.0_f32.to_bits()
            );
        }
    }

    #[test]
    fn mesh_storage_budget_matches_actual_recipes() {
        for level in 0..5 {
            for textured in [false, true] {
                let actual: usize = meshes(level, textured.then_some([10, 4]))
                    .iter()
                    .map(|m| {
                        m.vertices.len() * size_of::<Vertex>() + m.indices.len() * size_of::<u16>()
                    })
                    .sum();
                assert_eq!(mesh_bytes(level, textured), u64::try_from(actual).unwrap());
            }
        }
    }

    #[test]
    fn textured_caps_balls_and_uvs_follow_the_recovered_builders() {
        let short = tube(8, 5, Some([single(4.0 / 7.0), 2.0, 2.0]));
        assert_eq!(
            short.vertices[0].uv[0].to_bits(),
            single(4.0 / 7.0).to_bits()
        );
        assert_eq!(
            short.vertices.last().unwrap().uv.map(f32::to_bits),
            [2.0_f32; 2].map(f32::to_bits)
        );
        for level in 0..5 {
            for repeat in [[1, 1], [4, 2], [10, 4]] {
                let all = meshes(level, Some(repeat));
                assert_eq!(all.len(), 12);
                for mesh in &all {
                    assert!(
                        mesh.indices
                            .iter()
                            .all(|&i| usize::from(i) < mesh.vertices.len())
                    );
                    assert!(
                        mesh.vertices
                            .iter()
                            .all(|v| v.uv.into_iter().all(f32::is_finite))
                    );
                }
                for vertex in &all[7].vertices {
                    assert!((Vec3::from_array(vertex.position).length() - SQRT_2).abs() < 2e-5);
                }
                for mesh in &all[8..] {
                    for vertex in &mesh.vertices {
                        assert!(
                            ((Vec3::from_array(vertex.position) + Vec3::Z).length() - SQRT_2).abs()
                                < 2e-5
                        );
                        assert!((Vec3::from_array(vertex.normal).length() - 1.0).abs() < 1e-5);
                    }
                }
            }
        }
    }

    #[test]
    fn wrapping_uses_shortest_periodic_edges_and_preserves_half_ties() {
        let mut mesh = Mesh {
            vertices: [0.9, 0.1, 0.2]
                .map(|u| vertex(Vec3::ZERO, Vec3::Z, [u, 0.0]))
                .to_vec(),
            indices: vec![0, 1, 2],
        };
        wrap_coordinates(&mut mesh);
        assert!((mesh.vertices[1].uv[0] - 1.1).abs() < 1e-6);
        assert!((mesh.vertices[2].uv[0] - 1.2).abs() < 1e-6);
        let mut mesh = Mesh {
            vertices: [1.0, 1.5, 3.5]
                .map(|u| vertex(Vec3::ZERO, Vec3::Z, [u, 0.0]))
                .to_vec(),
            indices: vec![0, 1, 2],
        };
        wrap_coordinates(&mut mesh);
        assert_eq!(mesh.vertices[1].uv[0].to_bits(), 1.5_f32.to_bits());
        assert_eq!(mesh.vertices[2].uv[0].to_bits(), 1.5_f32.to_bits());
    }

    #[test]
    fn directed_transforms_and_ordered_spans() {
        for d in Direction::ALL {
            assert!((align_z(d) * Vec3::Z - vector(d)).length() < 1e-6);
            for n in Direction::ALL.into_iter().filter(|n| n.axis != d.axis) {
                assert!(
                    (align_z(d) * roll(d.roll_degrees(n)) * Vec3::Y - vector(n)).length() < 1e-6
                );
            }
        }
        let mut scene = Scene::default();
        let cap = Event::Joint {
            cell: [1; 3],
            color: 0,
        };
        let segment = Event::Segment {
            cell: [1; 3],
            direction: Direction::ALL[4],
            notch: Direction::ALL[2],
            long: false,
            color: 1,
        };
        scene.apply(&[cap, segment, segment, cap], [3; 3]);
        assert_eq!(
            scene
                .spans
                .iter()
                .map(|s| (s.primitive, s.start, s.count))
                .collect::<Vec<_>>(),
            [
                (Primitive::Ball, 0, 1),
                (Primitive::Short, 1, 2),
                (Primitive::Ball, 3, 1)
            ]
        );
        assert_eq!(
            scene.instances[1].center.map(f32::to_bits),
            [0.0_f32, 0.0, 1.0].map(f32::to_bits)
        );
        let capacity = scene.instances.capacity();
        scene.apply(&[Event::Reset, cap], [3; 3]);
        assert_eq!(scene.instances.len(), 1);
        assert_eq!(scene.instances.capacity(), capacity);
        assert_eq!(scene.generation, 1);
    }
}

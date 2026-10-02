//! Platform-neutral animation driver. Hosts supply monotonic time and process state.
use crate::{
    Result,
    config::Config,
    geometry::{self, Scene},
    rng::Random,
    simulation::{Cell, Event, Simulation},
    timing::{FadeClock, FadeStep, RenderGate},
};

pub(crate) struct Animation {
    pub(crate) dims: Cell,
    pub(crate) simulation: Simulation,
    pub(crate) scene: Scene,
    events: Vec<Event>,
    gate: RenderGate,
}

impl Animation {
    pub(crate) fn new(config: &Config, logical: [u32; 2]) -> Result<Self> {
        let dims = geometry::layout(logical)?;
        Ok(Self {
            dims,
            simulation: Simulation::new(
                dims,
                config.joints,
                config.pipes,
                config.textured(),
                config.scene_length,
            ),
            scene: Scene::default(),
            events: Vec::with_capacity(17),
            gate: RenderGate::new(config.speed),
        })
    }

    pub(crate) fn resize(&mut self, config: &Config, logical: [u32; 2]) -> Result<()> {
        let dims = geometry::layout(logical)?;
        if dims != self.dims {
            self.simulation = Simulation::new(
                dims,
                config.joints,
                config.pipes,
                config.textured(),
                config.scene_length,
            );
            self.dims = dims;
            self.scene.reset(false);
        }
        Ok(())
    }

    pub(crate) fn render(
        &mut self,
        now: f64,
        random: &mut Random,
        fade: &mut FadeClock,
    ) -> Result<bool> {
        if !self.gate.accepts(now) {
            return Ok(false);
        }
        if self.simulation.ended() {
            match fade.step(now) {
                FadeStep::Wait => return Ok(true),
                FadeStep::Overlay => {
                    self.scene.fades = self
                        .scene
                        .fades
                        .checked_add(1)
                        .ok_or("fade counter overflow")?;
                    return Ok(true);
                }
                FadeStep::Clear => {}
            }
        }
        self.simulation.tick(random, &mut self.events);
        self.scene.apply(&self.events, self.dims);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstructed_settings_matrix_replays_three_scene_resets() {
        use crate::config::{Joints, Pipes};
        let mut cases = 0;
        for viewport in [[640, 480], [5120, 2160], [2160, 3840]] {
            for seed in [0, 1, 42] {
                for pipes in [Pipes::Fixed(1), Pipes::Multiple] {
                    for textured in [false, true] {
                        for joints in [Joints::Elbow, Joints::Ball, Joints::Mixed, Joints::Cycle] {
                            let cfg = Config {
                                seed: Some(seed),
                                pipes,
                                joints,
                                texture: textured.then(|| "diagnostic.png".into()),
                                ..Config::default()
                            };
                            let (mut a, mut b) = (
                                Animation::new(&cfg, viewport).unwrap(),
                                Animation::new(&cfg, viewport).unwrap(),
                            );
                            let (mut ra, mut rb) = (Random::new(cfg.seed), Random::new(cfg.seed));
                            let (mut fa, mut fb) = (FadeClock::default(), FadeClock::default());
                            for frame in 0..30000 {
                                let (old_generation, old_count) =
                                    (a.scene.generation, a.scene.instances.len());
                                let now = f64::from(frame) * 0.025;
                                assert_eq!(
                                    a.render(now, &mut ra, &mut fa).unwrap(),
                                    b.render(now, &mut rb, &mut fb).unwrap()
                                );
                                assert_eq!(ra.checkpoint(), rb.checkpoint());
                                assert_eq!(
                                    (a.scene.generation, a.scene.fades, a.scene.instances.len()),
                                    (b.scene.generation, b.scene.fades, b.scene.instances.len())
                                );
                                let first = if old_generation == a.scene.generation {
                                    old_count
                                } else {
                                    0
                                };
                                let added = &a.scene.instances[first..];
                                assert_eq!(
                                    bytemuck::cast_slice::<_, u8>(added),
                                    bytemuck::cast_slice::<_, u8>(&b.scene.instances[first..])
                                );
                                assert!(added.iter().all(|i| {
                                    i.center.into_iter().chain(i.rotation).all(f32::is_finite)
                                }));
                                assert!(added.iter().all(|i| if textured {
                                    (24..28).contains(&i.material)
                                } else {
                                    i.material < 24
                                }));
                                assert!(a.scene.spans.last().is_none_or(|span| span.start
                                    + span.count
                                    == a.scene.instances.len()));
                                if a.scene.fades > 0 {
                                    assert_eq!(a.scene.instances.len(), old_count);
                                }
                                if a.scene.generation >= 3 {
                                    break;
                                }
                            }
                            assert_eq!(
                                a.scene.generation, 3,
                                "{viewport:?} seed={seed} pipes={pipes:?} textured={textured} joints={joints:?}"
                            );
                            cases += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(cases, 144);
    }
}

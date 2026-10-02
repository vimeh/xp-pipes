//! Explicitly requested, display-independent GLES test of the production shader.
//! This verifies the reconstructed equation, not pixels from a running XP binary.
use super::*;
#[cfg(target_os = "macos")]
use crate::macos::Cgl as Offscreen;
use glam::{Mat4, Vec3};
#[cfg(target_os = "linux")]
use khronos_egl as egl;

#[cfg(target_os = "linux")]
struct Offscreen {
    api: egl::Instance<egl::Static>,
    display: egl::Display,
    context: Option<egl::Context>,
    surface: Option<egl::Surface>,
}
#[cfg(target_os = "linux")]
impl Offscreen {
    fn load_gl(&self) -> Result<glow::Context> {
        self.api
            .make_current(self.display, self.surface, self.surface, self.context)?;
        // SAFETY: this test's owned EGL context is current.
        Ok(unsafe {
            glow::Context::from_loader_function(|name| {
                self.api
                    .get_proc_address(name)
                    .map_or(std::ptr::null(), |f| (f as *const ()).cast())
            })
        })
    }

    fn new() -> Result<Self> {
        let api = egl::Instance::new(egl::Static);
        // SAFETY: DEFAULT_DISPLAY is an EGL-defined native handle. Tests select
        // the surfaceless Mesa platform through their process environment.
        let display =
            unsafe { api.get_display(egl::DEFAULT_DISPLAY) }.ok_or("no test EGL display")?;
        api.initialize(display)?;
        let mut result = Self {
            api,
            display,
            context: None,
            surface: None,
        };
        result.api.bind_api(egl::OPENGL_ES_API)?;
        let config = result
            .api
            .choose_first_config(
                display,
                &[
                    egl::SURFACE_TYPE,
                    egl::PBUFFER_BIT,
                    egl::RENDERABLE_TYPE,
                    egl::OPENGL_ES3_BIT,
                    egl::RED_SIZE,
                    8,
                    egl::GREEN_SIZE,
                    8,
                    egl::BLUE_SIZE,
                    8,
                    egl::DEPTH_SIZE,
                    24,
                    egl::NONE,
                ],
            )?
            .ok_or("no test GLES config")?;
        result.context = Some(result.api.create_context(
            display,
            config,
            None,
            &[egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE],
        )?);
        result.surface = Some(result.api.create_pbuffer_surface(
            display,
            config,
            &[egl::WIDTH, 32, egl::HEIGHT, 32, egl::NONE],
        )?);
        result
            .api
            .make_current(display, result.surface, result.surface, result.context)?;
        Ok(result)
    }
}
#[cfg(target_os = "linux")]
impl Drop for Offscreen {
    fn drop(&mut self) {
        let _ = self.api.make_current(self.display, None, None, None);
        if let Some(surface) = self.surface {
            let _ = self.api.destroy_surface(self.display, surface);
        }
        if let Some(context) = self.context {
            let _ = self.api.destroy_context(self.display, context);
        }
        let _ = self.api.terminate(self.display);
    }
}

#[test]
#[ignore = "requires a native GL driver (surfaceless Mesa on Linux); not an XP runtime oracle"]
fn gpu_vertex_lighting_matches_reference_equation() {
    let context = Offscreen::new().unwrap();
    // SAFETY: the test context is current on this thread. All GL objects remain
    // owned by that context, including on panic; mapped bytes are checked below.
    unsafe {
        let gl = Rc::new(context.load_gl().unwrap());
        // CGL has no pbuffer on Apple Silicon. Exercise the same operations on
        // an explicit FBO there, retaining Linux's default-framebuffer coverage.
        #[cfg(target_os = "macos")]
        let presentation = History::new(gl.clone(), None, [32; 2]).unwrap();
        #[cfg(target_os = "macos")]
        let destination = presentation.framebuffer;
        #[cfg(target_os = "linux")]
        let destination = None;
        eprintln!(
            "GLES test renderer: {}",
            gl.get_parameter_string(glow::RENDERER)
        );
        let program = pipe_program(&gl, true).unwrap();
        gl.use_program(Some(program));
        for name in ["u_view", "u_projection"] {
            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(program, name).as_ref(),
                false,
                &Mat4::IDENTITY.to_cols_array(),
            );
        }
        let vao = gl.create_vertex_array().unwrap();
        gl.bind_vertex_array(Some(vao));
        gl.vertex_attrib_3_f32(2, 0.0, 0.0, 0.0);
        gl.vertex_attrib_4_f32(3, 0.0, 0.0, 0.0, 1.0);
        let buffer = gl.create_buffer().unwrap();
        gl.bind_buffer(glow::TRANSFORM_FEEDBACK_BUFFER, Some(buffer));
        gl.buffer_data_size(glow::TRANSFORM_FEEDBACK_BUFFER, 24, glow::DYNAMIC_READ);
        gl.bind_buffer_base(glow::TRANSFORM_FEEDBACK_BUFFER, 0, Some(buffer));
        gl.enable(glow::RASTERIZER_DISCARD);
        for (material, position, normal) in [
            (0_u8, Vec3::new(0.0, 0.0, 75.0), -Vec3::Z),
            (10, Vec3::new(20.0, -10.0, 50.0), Vec3::new(0.3, 0.2, -0.9)),
            (16, Vec3::new(0.0, 0.0, 75.0), Vec3::Z),
            (3, Vec3::new(0.0, 0.0, 75.0), Vec3::new(0.0, 50.0, -150.0)),
        ] {
            gl.vertex_attrib_3_f32(0, position.x, position.y, position.z);
            gl.vertex_attrib_3_f32(1, normal.x, normal.y, normal.z);
            gl.vertex_attrib_4_u32(4, u32::from(material), 0, 0, 1);
            gl.begin_transform_feedback(glow::POINTS);
            gl.draw_arrays(glow::POINTS, 0, 1);
            gl.end_transform_feedback();
            let pointer =
                gl.map_buffer_range(glow::TRANSFORM_FEEDBACK_BUFFER, 0, 24, glow::MAP_READ_BIT);
            assert!(!pointer.is_null(), "map failed: {}", gl.get_error());
            let bytes = std::slice::from_raw_parts(pointer, 24);
            let actual: Vec<_> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_ne_bytes(*b))
                .collect();
            gl.unmap_buffer(glow::TRANSFORM_FEEDBACK_BUFFER);
            let row = materials::record(usize::from(material));
            let ambient = Vec3::from_slice(&row[..3]);
            let diffuse = Vec3::from_slice(&row[3..6]);
            let specular = Vec3::from_slice(&row[6..9]);
            let light = Vec3::new(0.0, 50.0, -150.0).normalize();
            let cosine = normal.normalize().dot(light).max(0.0);
            let expected_diffuse = (ambient * 1.1 + diffuse * cosine).clamp(Vec3::ZERO, Vec3::ONE);
            let highlight = if cosine > 0.0 {
                normal
                    .normalize()
                    .dot((light - position.normalize()).normalize())
                    .max(0.0)
                    .powf(row[9] * 128.0)
            } else {
                0.0
            };
            let expected_specular = specular * 0.6 * highlight;
            for (observed, expected) in actual.into_iter().zip(
                expected_diffuse
                    .to_array()
                    .into_iter()
                    .chain(expected_specular.to_array()),
            ) {
                assert!(
                    (observed - expected).abs() < 2e-5,
                    "material {material}: GPU={observed}, expected={expected}"
                );
            }
            assert_eq!(gl.get_error(), glow::NO_ERROR);
        }
        gl.disable(glow::RASTERIZER_DISCARD);
        assert_depth_and_culling(&gl);
        gl.delete_buffer(buffer);
        gl.delete_vertex_array(vao);
        gl.delete_program(program);
        assert_history_replay(&gl, destination);
        assert_texture_sampling(&gl, destination);
        assert_large_histories(&gl);
        assert_rendering_matrix(&gl);
    }
}

// SAFETY: the test owns one current context. Cases pin viewport, transforms and
// material/mesh choices, rather than taking screenshots of wall-clock animation.
unsafe fn assert_rendering_matrix(gl: &Rc<glow::Context>) {
    unsafe {
        let texture = Texture {
            size: [2, 2],
            rgba: vec![
                220, 40, 70, 255, 40, 230, 80, 255, 50, 80, 240, 255, 240, 220, 50, 255,
            ],
        };
        let mut count = 0;
        for level in 0..5 {
            for textured in [false, true] {
                let mut renderer =
                    Renderer::from_gl(gl.clone(), level, textured.then_some(&texture)).unwrap();
                let mut history = History::new(gl.clone(), None, [32; 2]).unwrap();
                let mut scene = Scene::default();
                let solid = [
                    geometry::Primitive::Short,
                    geometry::Primitive::Long,
                    geometry::Primitive::Ball,
                    geometry::Primitive::Elbow0,
                    geometry::Primitive::Elbow1,
                    geometry::Primitive::Elbow2,
                    geometry::Primitive::Elbow3,
                ];
                let textured_meshes = [
                    geometry::Primitive::Short,
                    geometry::Primitive::Long,
                    geometry::Primitive::TexturedCap,
                    geometry::Primitive::Elbow0,
                    geometry::Primitive::Elbow1,
                    geometry::Primitive::Elbow2,
                    geometry::Primitive::Elbow3,
                    geometry::Primitive::TexturedBall0,
                    geometry::Primitive::TexturedBall1,
                    geometry::Primitive::TexturedBall2,
                    geometry::Primitive::TexturedBall3,
                ];
                let meshes = if textured {
                    &textured_meshes[..]
                } else {
                    &solid[..]
                };
                let materials = if textured {
                    &[24_u8, 25, 26, 27][..]
                } else {
                    &materials::GOOD[..]
                };
                for &primitive in meshes {
                    for &material in materials {
                        scene.reset(false);
                        scene.push(
                            primitive,
                            Instance {
                                center: [0.0, 0.0, 60.0],
                                // View the outside of open tubes, not down their
                                // deliberately back-face-culled empty interior.
                                rotation: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
                                    .to_array(),
                                material: u32::from(material),
                            },
                        );
                        renderer.update_history(&mut history, &scene).unwrap();
                        assert!(
                            pixels(gl, history.framebuffer)
                                .as_chunks::<4>()
                                .0
                                .iter()
                                .any(|p| p[..3].iter().any(|&v| v > 5)),
                            "blank mesh/material case: level={level} textured={textured} {primitive:?} material={material}"
                        );
                        count += 1;
                    }
                }
            }
        }
        assert_eq!(count, 780);
        eprintln!("Fixed-scene GLES mesh/material matrix: {count} cases");
    }
}

// SAFETY: explicit software-GLES test context is current, with no live desktop.
unsafe fn assert_large_histories(gl: &Rc<glow::Context>) {
    unsafe {
        let mut renderer = Renderer::from_gl(gl.clone(), 0, None).unwrap();
        for size in [
            [1920, 1080],
            [3440, 1440],
            [5120, 2160],
            [3840, 2160],
            [7680, 4320],
            [2160, 3840],
        ] {
            let mut scene = Scene::default();
            let mut history = History::new(gl.clone(), None, size).unwrap();
            renderer.update_history(&mut history, &scene).unwrap();
            gl.bind_renderbuffer(glow::RENDERBUFFER, history.depth);
            assert_eq!(
                gl.get_renderbuffer_parameter_i32(glow::RENDERBUFFER, glow::RENDERBUFFER_WIDTH),
                size[0]
            );
            assert_eq!(
                gl.get_renderbuffer_parameter_i32(glow::RENDERBUFFER, glow::RENDERBUFFER_HEIGHT),
                size[1]
            );
            let mut pixel = [0; 4];
            gl.read_pixels(
                size[0] / 2,
                size[1] / 2,
                1,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixel)),
            );
            assert_eq!(pixel, [0, 0, 0, 255]);
            scene.push(
                geometry::Primitive::Ball,
                Instance {
                    center: [0.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    material: 4,
                },
            );
            renderer.update_history(&mut history, &scene).unwrap();
            gl.read_pixels(
                size[0] / 2,
                size[1] / 2,
                1,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixel)),
            );
            assert!(
                pixel[0] > pixel[1] && pixel[0] > 70,
                "large target did not render geometry: {size:?} {pixel:?}"
            );
            eprintln!("Native-size GLES target rendered: {size:?}");
            let framebuffer = history.framebuffer.unwrap();
            gl.finish(); // Bound pending driver work/memory between large allocations.
            drop(history);
            assert!(!gl.is_framebuffer(framebuffer));
            assert_eq!(gl.get_error(), glow::NO_ERROR);
        }
        assert!(History::new(gl.clone(), None, [16385, 1]).is_err());
        assert!(History::new(gl.clone(), None, [-1, 1]).is_err());
    }
}

// SAFETY: the outer test owns the current EGL context throughout these draws.
unsafe fn assert_texture_sampling(gl: &Rc<glow::Context>, destination: Option<glow::Framebuffer>) {
    unsafe {
        if let Some(path) = std::env::var_os("WLPIPES_REFERENCE_TEXTURE") {
            let original = Texture::load(Some(std::path::Path::new(&path)), None)
                .unwrap()
                .unwrap();
            assert_eq!(original.size, [64, 64]);
            let name = upload_texture(gl, &original).unwrap();
            gl.delete_texture(name);
            eprintln!(
                "Supplied private 64x64 reference bitmap decoded/uploaded; not an XP pixel comparison"
            );
        }
        let image = Texture {
            size: [2, 2],
            rgba: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        };
        let mut renderer = Renderer::from_gl(gl.clone(), 0, Some(&image)).unwrap();
        let mut history = History::new(gl.clone(), None, [32; 2]).unwrap();
        let mut scene = Scene::default();
        scene.push(
            geometry::Primitive::TexturedCap,
            Instance {
                center: [0.0, 0.0, 70.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                material: 24,
            },
        );
        renderer.update_history(&mut history, &scene).unwrap();
        assert_eq!(history.meshes.len(), 12);
        let pixels = pixels(gl, history.framebuffer);
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[0] > p[1] && p[0] > 30)
        );
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[1] > p[0] && p[1] > 30)
        );
        let name = history.meshes[0].vertices;
        drop(history);
        assert!(
            !gl.is_buffer(name),
            "resized/retired textured meshes leaked"
        );
        let bad = Texture {
            size: [2, 2],
            rgba: vec![0; 4],
        };
        assert!(upload_texture(gl, &bad).is_err());
        assert_transparent_depth(gl, &image, destination);
    }
}

// SAFETY: current test context; all uploaded arrays have checked byte sizes.
unsafe fn assert_transparent_depth(
    gl: &glow::Context,
    image: &Texture,
    destination: Option<glow::Framebuffer>,
) {
    unsafe {
        let program = pipe_program(gl, false).unwrap();
        gl.use_program(Some(program));
        for name in ["u_view", "u_projection"] {
            gl.uniform_matrix_4_f32_slice(
                gl.get_uniform_location(program, name).as_ref(),
                false,
                &Mat4::IDENTITY.to_cols_array(),
            );
        }
        gl.bind_framebuffer(glow::FRAMEBUFFER, destination);
        let vao = gl.create_vertex_array().unwrap();
        gl.bind_vertex_array(Some(vao));
        let buffer = gl.create_buffer().unwrap();
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
        gl.vertex_attrib_3_f32(1, 0.0, 50.0, -150.0);
        gl.vertex_attrib_3_f32(2, 0.0, 0.0, 0.0);
        gl.vertex_attrib_4_f32(3, 0.0, 0.0, 0.0, 1.0);
        gl.vertex_attrib_4_u32(4, 24, 0, 0, 1);
        gl.vertex_attrib_2_f32(5, 1.25, 0.25); // Repeat-addressed first texel centre.
        let opaque = upload_texture(gl, image).unwrap();
        pipe_state(gl);
        gl.viewport(0, 0, 32, 32);
        gl.clear_depth_f32(1.0);
        gl.clear_color(0.0, 0.0, 1.0, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        let vertices = [[-0.9_f32, -0.9, 0.25], [0.9, -0.9, 0.25], [0.0, 0.9, 0.25]];
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(&vertices),
            glow::STATIC_DRAW,
        );
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        let mut pixel = [0; 4];
        gl.read_pixels(
            16,
            16,
            1,
            1,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixel)),
        );
        assert_eq!(
            pixel,
            [255, 0, 0, 255],
            "texture orientation/repeat/modulation differs"
        );
        let transparent = Texture {
            size: [2, 2],
            rgba: [255, 0, 0, 0].repeat(4),
        };
        let name = upload_texture(gl, &transparent).unwrap();
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.vertex_attrib_4_u32(4, 16, 0, 0, 1);
        let farther = [[-0.9_f32, -0.9, 0.5], [0.9, -0.9, 0.5], [0.0, 0.9, 0.5]];
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(&farther),
            glow::STATIC_DRAW,
        );
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.read_pixels(
            16,
            16,
            1,
            1,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixel)),
        );
        assert_eq!(
            pixel,
            [0, 0, 255, 255],
            "alpha=0 must blend away but retain XP's depth writes"
        );
        assert_eq!(gl.get_error(), glow::NO_ERROR);
        gl.delete_texture(name);
        gl.delete_texture(opaque);
        gl.delete_buffer(buffer);
        gl.delete_vertex_array(vao);
        gl.delete_program(program);
    }
}

// SAFETY: the caller owns the current test context and the selected framebuffer.
unsafe fn pixels(gl: &glow::Context, framebuffer: Option<glow::Framebuffer>) -> Vec<u8> {
    let mut data = vec![0; 32 * 32 * 4];
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, framebuffer);
        gl.read_pixels(
            0,
            0,
            32,
            32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut data)),
        );
        assert_eq!(gl.get_error(), glow::NO_ERROR);
    }
    data
}

// SAFETY: all histories are dropped before the outer EGL context, while current.
unsafe fn assert_history_replay(gl: &Rc<glow::Context>, destination: Option<glow::Framebuffer>) {
    unsafe {
        let mut renderer = Renderer::from_gl(gl.clone(), 0, None).unwrap();
        let mut first = History::new(gl.clone(), None, [32; 2]).unwrap();
        let mut second = History::new(gl.clone(), None, [32; 2]).unwrap();
        let mut scene = Scene::default();
        let instance = Instance {
            center: [0.0, 0.0, 70.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            material: 4,
        };
        scene.push(geometry::Primitive::Ball, instance);
        renderer.update_history(&mut first, &scene).unwrap();
        let initial = pixels(gl, first.framebuffer);
        assert!(
            initial
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[0] > 100 && p[0] > p[1])
        );
        assert!(initial.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        let mut white = Scene::default();
        white.push(
            geometry::Primitive::Ball,
            Instance {
                material: 16,
                ..instance
            },
        );
        renderer.update_history(&mut second, &white).unwrap();
        assert_ne!(pixels(gl, second.framebuffer), initial);
        renderer.update_history(&mut first, &scene).unwrap();
        assert_eq!(
            pixels(gl, first.framebuffer),
            initial,
            "another output must not overwrite an unchanged history"
        );
        scene.push(
            geometry::Primitive::Ball,
            Instance {
                center: [2.0, 0.0, 70.0],
                material: 10,
                ..instance
            },
        );
        renderer.update_history(&mut first, &scene).unwrap();
        let mut replay = History::new(gl.clone(), None, [32; 2]).unwrap();
        renderer.update_history(&mut replay, &scene).unwrap();
        assert_eq!(
            pixels(gl, first.framebuffer),
            pixels(gl, replay.framebuffer),
            "incremental and fresh replay differ"
        );
        let previous = assert_fade_steps(&mut renderer, &mut first, &mut scene);
        let mut cold = History::new(gl.clone(), None, [32; 2]).unwrap();
        renderer.update_history(&mut cold, &scene).unwrap();
        assert_eq!(
            pixels(gl, cold.framebuffer),
            previous,
            "cold replay lost fade rounding/history"
        );
        renderer.present(&first, destination).unwrap();
        assert_eq!(
            pixels(gl, destination),
            previous,
            "window copy changed colour or opacity"
        );
        scene.reset(true);
        renderer.update_history(&mut first, &scene).unwrap();
        assert!(
            pixels(gl, first.framebuffer)
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [0, 0, 0, 255])
        );
        scene.push(geometry::Primitive::Ball, instance);
        renderer.update_history(&mut first, &scene).unwrap();
        assert_eq!(
            pixels(gl, first.framebuffer),
            initial,
            "reset did not clear old depth/fade state"
        );
        let name = cold.framebuffer.unwrap();
        drop(cold);
        assert!(!gl.is_framebuffer(name));
        assert!(History::new(gl.clone(), None, [0, 32]).is_err());
        assert!(history_bytes([16384; 2]).is_err());
        assert_eq!(history_bytes([5120, 2160]).unwrap(), 88_473_600);
    }
}

// SAFETY: the caller keeps the test context current and all resources alive.
unsafe fn assert_fade_steps(
    renderer: &mut Renderer,
    history: &mut History,
    scene: &mut Scene,
) -> Vec<u8> {
    let gl = renderer.gl.clone();
    unsafe {
        let mut previous = pixels(&gl, history.framebuffer);
        for pass in 1..=29 {
            scene.fades = pass;
            renderer.update_history(history, scene).unwrap();
            let faded = pixels(&gl, history.framebuffer);
            for (before, after) in previous
                .as_chunks::<4>()
                .0
                .iter()
                .zip(faded.as_chunks::<4>().0)
            {
                for channel in 0..3 {
                    let expected = (u32::from(before[channel]) * 223 + 127) / 255;
                    assert!(
                        expected.abs_diff(u32::from(after[channel])) <= 1,
                        "fade is not quantized alpha=32/255 blending"
                    );
                }
                assert_eq!(after[3], 255, "history/presentation must remain opaque");
            }
            renderer.update_history(history, scene).unwrap();
            assert_eq!(
                pixels(&gl, history.framebuffer),
                faded,
                "presenting again must not apply another fade"
            );
            previous = faded;
        }
        previous
    }
}

// SAFETY: caller owns the current context, bound production program and VAO.
unsafe fn assert_depth_and_culling(gl: &glow::Context) {
    unsafe {
        pipe_state(gl);
        gl.viewport(0, 0, 32, 32);
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear_depth_f32(1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        gl.vertex_attrib_3_f32(1, 0.0, 0.0, -1.0);
        let buffer = gl.create_buffer().unwrap();
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
        let mut previous = [0; 4];
        for (index, material, depth, reverse) in [
            (0, 16, 0.5, false),
            (1, 4, 0.5, false),
            (2, 16, 0.75, false),
            (3, 16, 0.25, true),
        ] {
            let mut vertices = [
                [-0.9_f32, -0.9, depth],
                [0.9, -0.9, depth],
                [0.0, 0.9, depth],
            ];
            if reverse {
                vertices.swap(0, 1);
            }
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&vertices),
                glow::STREAM_DRAW,
            );
            gl.vertex_attrib_4_u32(4, material, 0, 0, 1);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            let mut pixel = [0; 4];
            gl.read_pixels(
                16,
                16,
                1,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixel)),
            );
            match index {
                0 => assert!(
                    pixel[0] > 20 && pixel[0] == pixel[1],
                    "front face must render white: {pixel:?}"
                ),
                1 => assert!(
                    pixel[0] > pixel[1] + 20,
                    "equal-depth red must replace white: {pixel:?}"
                ),
                _ => assert_eq!(
                    pixel, previous,
                    "farther or back-facing geometry must not replace red"
                ),
            }
            previous = pixel;
            assert_eq!(gl.get_error(), glow::NO_ERROR);
        }
        gl.delete_buffer(buffer);
    }
}

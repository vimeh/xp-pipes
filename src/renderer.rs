//! Incremental colour/depth histories per output, shared meshes and programs.
use crate::{
    Result,
    geometry::{self, Instance, Mesh, Scene, Vertex},
    materials,
    texture::{self, Texture},
};
use glow::HasContext;
use std::rc::Rc;

/// Native adapters retain their context/drawable until all histories are gone.
pub(crate) trait RenderSurface {
    fn make_current(&self) -> Result<()>;
    fn load_gl(&self) -> Result<glow::Context>;
}

pub struct Renderer {
    gl: Rc<glow::Context>,
    program: glow::Program,
    fade_program: glow::Program,
    fade_vao: glow::VertexArray,
    view: glow::UniformLocation,
    projection: glow::UniformLocation,
    meshes: Vec<GpuMesh>,
    texture: Option<glow::Texture>,
    texture_size: Option<[u32; 2]>,
    tessellation: u8,
    instances: glow::Buffer,
    capacity: usize,
}

/// Own the drawable until after target deletion, so Drop can restore its context
/// even if dropping another output previously unbound the shared EGL context.
pub struct History {
    gl: Rc<glow::Context>,
    drawable: Option<Rc<dyn RenderSurface>>, // None only in the explicit offscreen GPU test.
    framebuffer: Option<glow::Framebuffer>,
    color: Option<glow::Texture>,
    depth: Option<glow::Renderbuffer>,
    pub size: [i32; 2],
    generation: Option<u64>,
    drawn: usize,
    faded: u32,
    meshes: Vec<GpuMesh>,
}

pub fn history_bytes(size: [i32; 2]) -> Result<u64> {
    if size.into_iter().any(|n| !(1..=16384).contains(&n)) {
        return Err("invalid history dimensions".into());
    }
    let pixels = u64::try_from(size[0])? * u64::try_from(size[1])?;
    if pixels > 64 * 1024 * 1024 {
        return Err("history exceeds 64 megapixels".into());
    }
    // Account for padded RGB8 and padded D16 driver storage, not just 3+2 bytes.
    Ok(pixels * 8)
}

impl History {
    // SAFETY: caller has made this GL context current; offscreen tests keep their
    // owning EGL context alive until this object has been destroyed.
    unsafe fn new(
        gl: Rc<glow::Context>,
        drawable: Option<Rc<dyn RenderSurface>>,
        size: [i32; 2],
    ) -> Result<Self> {
        history_bytes(size)?;
        let mut result = Self {
            gl,
            drawable,
            framebuffer: None,
            color: None,
            depth: None,
            size,
            generation: None,
            drawn: 0,
            faded: 0,
            meshes: Vec::new(),
        };
        unsafe {
            let gl = &result.gl;
            let limit = gl
                .get_parameter_i32(glow::MAX_TEXTURE_SIZE)
                .min(gl.get_parameter_i32(glow::MAX_RENDERBUFFER_SIZE));
            let mut viewport = [0; 2];
            gl.get_parameter_i32_slice(glow::MAX_VIEWPORT_DIMS, &mut viewport);
            if size[0] > limit.min(viewport[0]) || size[1] > limit.min(viewport[1]) {
                return Err("history dimensions exceed GLES capabilities".into());
            }
            result.framebuffer = Some(gl.create_framebuffer()?);
            result.color = Some(gl.create_texture()?);
            result.depth = Some(gl.create_renderbuffer()?);
            gl.bind_texture(glow::TEXTURE_2D, result.color);
            for (parameter, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, i32::try_from(value)?);
            }
            // RGB8 deliberately has implicit alpha=1. Fading cannot make a
            // Wayland background translucent if EGL selects an RGBA window.
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                i32::try_from(glow::RGB8)?,
                size[0],
                size[1],
                0,
                glow::RGB,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.bind_renderbuffer(glow::RENDERBUFFER, result.depth);
            // FindDepthStencilFormat first selects D16 with the base 16/0 minima.
            gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::DEPTH_COMPONENT16,
                size[0],
                size[1],
            );
            gl.bind_framebuffer(glow::FRAMEBUFFER, result.framebuffer);
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                result.color,
                0,
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                result.depth,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            let error = gl.get_error();
            if status != glow::FRAMEBUFFER_COMPLETE || error != glow::NO_ERROR {
                return Err(format!(
                    "history allocation failed: framebuffer={status:#x}, GL={error:#x}"
                )
                .into());
            }
        }
        Ok(result)
    }
}
impl Drop for History {
    fn drop(&mut self) {
        if self
            .drawable
            .as_ref()
            .is_some_and(|d| d.make_current().is_err())
        {
            // Context loss: its eventual EGL teardown owns the remaining names.
            return;
        }
        // SAFETY: an owned drawable restored the context (or the GPU test owns
        // its current offscreen context). Options also clean partial allocation.
        unsafe {
            for mesh in &self.meshes {
                mesh.delete(&self.gl);
            }
            if let Some(f) = self.framebuffer {
                self.gl.delete_framebuffer(f);
            }
            if let Some(d) = self.depth {
                self.gl.delete_renderbuffer(d);
            }
            if let Some(c) = self.color {
                self.gl.delete_texture(c);
            }
        }
    }
}

// SAFETY: caller owns the current context. Feedback captures production outputs.
unsafe fn program(
    gl: &glow::Context,
    vertex: &str,
    fragment: &str,
    feedback: bool,
) -> Result<glow::Program> {
    unsafe {
        let program = gl.create_program()?;
        for (kind, source) in [
            (glow::VERTEX_SHADER, vertex),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = gl.create_shader(kind)?;
            // One shader body; desktop GL uses its native version directive.
            let desktop;
            let source = if gl.version().is_embedded {
                source
            } else {
                desktop = source.replacen("#version 300 es", "#version 410 core", 1);
                &desktop
            };
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(format!("GLES shader: {}", gl.get_shader_info_log(shader)).into());
            }
            gl.attach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if feedback {
            gl.transform_feedback_varyings(
                program,
                &["v_diffuse", "v_specular"],
                glow::INTERLEAVED_ATTRIBS,
            );
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!("GLES link: {}", gl.get_program_info_log(program)).into());
        }
        Ok(program)
    }
}

// SAFETY: caller has made the owning context current.
unsafe fn pipe_program(gl: &glow::Context, feedback: bool) -> Result<glow::Program> {
    unsafe {
        let program = program(
            gl,
            include_str!("shaders/pipe.vert"),
            include_str!("shaders/pipe.frag"),
            feedback,
        )?;
        gl.use_program(Some(program));
        gl.uniform_1_i32(gl.get_uniform_location(program, "u_texture").as_ref(), 0);
        let data = materials::table();
        for (name, values) in [
            ("u_ambient[0]", data.map(|m| m.ambient)),
            ("u_diffuse[0]", data.map(|m| m.diffuse)),
        ] {
            let location = gl
                .get_uniform_location(program, name)
                .ok_or_else(|| format!("missing material uniform {name}"))?;
            gl.uniform_3_f32_slice(Some(&location), values.as_flattened());
        }
        let specular = gl
            .get_uniform_location(program, "u_specular_power[0]")
            .ok_or("missing specular uniform")?;
        gl.uniform_4_f32_slice(
            Some(&specular),
            data.map(|m| m.specular_power).as_flattened(),
        );
        Ok(program)
    }
}

// SAFETY: caller owns the current context. Length and dimensions are checked
// before passing the pixel pointer to GL; decoder output is not blindly trusted.
unsafe fn upload_texture(gl: &glow::Context, texture: &Texture) -> Result<glow::Texture> {
    let [width, height] = texture.size;
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(4))
        .ok_or("texture size overflow")?;
    if width == 0
        || height == 0
        || bytes > 64 * 1024 * 1024
        || bytes != u64::try_from(texture.rgba.len())?
    {
        return Err("invalid decoded texture dimensions/length".into());
    }
    unsafe {
        let limit = u32::try_from(gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE))?;
        if width > limit || height > limit {
            return Err("texture exceeds GLES size capability".into());
        }
        let name = gl.create_texture()?;
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(name));
        for (parameter, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::REPEAT),
            (glow::TEXTURE_WRAP_T, glow::REPEAT),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, i32::try_from(value)?);
        }
        // XP leaves MIPFILTER at NONE. Do not add a different automatic mip filter.
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_BASE_LEVEL, 0);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAX_LEVEL, 0);
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            i32::try_from(glow::RGBA8)?,
            i32::try_from(width)?,
            i32::try_from(height)?,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&texture.rgba)),
        );
        let error = gl.get_error();
        if error != glow::NO_ERROR {
            gl.delete_texture(name);
            return Err(format!("texture upload GL error {error:#x}").into());
        }
        Ok(name)
    }
}

// SAFETY: the owning context must be current.
unsafe fn pipe_state(gl: &glow::Context) {
    unsafe {
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LEQUAL);
        gl.depth_mask(true);
        gl.enable(glow::CULL_FACE);
        gl.front_face(glow::CCW);
        gl.cull_face(glow::BACK);
        gl.enable(glow::BLEND);
        gl.blend_equation(glow::FUNC_ADD);
        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        gl.enable(glow::DITHER);
    }
}

#[cfg(test)]
#[path = "renderer_tests.rs"]
mod tests;

struct GpuMesh {
    vao: glow::VertexArray,
    vertices: glow::Buffer,
    indices: glow::Buffer,
    index_count: i32,
}

impl Renderer {
    /// Release fixed resources when a native host reuses its context after stop.
    /// # Safety
    /// The context is current; call exactly once, after retiring histories, and
    /// do not use the renderer afterward. Linux frees these with its sole context.
    #[cfg(target_os = "macos")]
    pub(crate) unsafe fn delete_fixed(&self) {
        unsafe {
            self.gl.use_program(None);
            for mesh in &self.meshes {
                mesh.delete(&self.gl);
            }
            if let Some(texture) = self.texture {
                self.gl.delete_texture(texture);
            }
            self.gl.delete_buffer(self.instances);
            self.gl.delete_vertex_array(self.fade_vao);
            self.gl.delete_program(self.fade_program);
            self.gl.delete_program(self.program);
        }
    }

    pub fn retained_bytes(&self) -> usize {
        self.capacity
    }

    pub fn fixed_gpu_bytes(&self) -> u64 {
        self.texture_size.map_or_else(
            || geometry::mesh_bytes(self.tessellation, false),
            |[width, height]| u64::from(width) * u64::from(height) * 4,
        )
    }

    pub fn new(
        target: &dyn RenderSurface,
        tessellation: u8,
        texture: Option<&Texture>,
    ) -> Result<Self> {
        let gl = Rc::new(target.load_gl()?);
        // SAFETY: load_gl made the owning context current.
        unsafe { Self::from_gl(gl, tessellation, texture) }
    }

    // SAFETY: shared constructor for the current Wayland or test GLES context.
    unsafe fn from_gl(
        gl: Rc<glow::Context>,
        tessellation: u8,
        texture: Option<&Texture>,
    ) -> Result<Self> {
        unsafe {
            let program = pipe_program(&gl, false)?;
            let view = gl
                .get_uniform_location(program, "u_view")
                .ok_or("missing view uniform")?;
            let projection = gl
                .get_uniform_location(program, "u_projection")
                .ok_or("missing projection uniform")?;
            let meshes = if texture.is_some() {
                Vec::new()
            } else {
                geometry::meshes(tessellation, None)
                    .iter()
                    .map(|m| GpuMesh::new(&gl, m))
                    .collect::<Result<Vec<_>>>()?
            };
            let texture_size = texture.map(|t| t.size);
            let texture = texture.map(|t| upload_texture(&gl, t)).transpose()?;
            let instances = gl.create_buffer()?;
            let fade_program = crate::renderer::program(
                &gl,
                include_str!("shaders/fade.vert"),
                include_str!("shaders/fade.frag"),
                false,
            )?;
            let fade_vao = gl.create_vertex_array()?;
            Ok(Self {
                gl,
                program,
                fade_program,
                fade_vao,
                view,
                projection,
                meshes,
                texture,
                texture_size,
                tessellation,
                instances,
                capacity: 0,
            })
        }
    }

    pub fn draw(
        &mut self,
        target: Rc<dyn RenderSurface>,
        history: &mut Option<History>,
        size: [i32; 2],
        scene: &Scene,
    ) -> Result<()> {
        target.make_current()?;
        if history.as_ref().is_some_and(|h| h.size != size) {
            *history = None;
        }
        // SAFETY: target made our context current. History retains it for Drop.
        unsafe {
            if history.is_none() {
                *history = Some(History::new(self.gl.clone(), Some(target), size)?);
            }
            let history = history.as_mut().unwrap();
            self.update_history(history, scene)?;
            self.present(history, None)?;
        }
        Ok(())
    }

    // SAFETY: history and the default framebuffer belong to the current context.
    unsafe fn present(
        &self,
        history: &History,
        destination: Option<glow::Framebuffer>,
    ) -> Result<()> {
        let [width, height] = history.size;
        unsafe {
            self.gl
                .bind_framebuffer(glow::READ_FRAMEBUFFER, history.framebuffer);
            self.gl
                .bind_framebuffer(glow::DRAW_FRAMEBUFFER, destination);
            self.gl.blit_framebuffer(
                0,
                0,
                width,
                height,
                0,
                0,
                width,
                height,
                glow::COLOR_BUFFER_BIT,
                glow::NEAREST,
            );
            let error = self.gl.get_error();
            if error != glow::NO_ERROR {
                return Err(format!("history presentation GL error {error:#x}").into());
            }
        }
        Ok(())
    }

    // SAFETY: history owns the current context. UV repetition is viewport-specific
    // in XP, so textured meshes are retired with that output, not cached forever.
    unsafe fn prepare_meshes(&self, history: &mut History) -> Result<()> {
        unsafe {
            if let Some(texture_size) = self.texture_size.filter(|_| history.meshes.is_empty()) {
                let repeat = texture::repeats(
                    history.size.map(|n| u32::try_from(n).unwrap()),
                    texture_size,
                );
                for mesh in geometry::meshes(self.tessellation, Some(repeat)) {
                    history.meshes.push(GpuMesh::new(&self.gl, &mesh)?);
                }
            }
        }
        Ok(())
    }

    // SAFETY: the history's owning context is current. Rebuild/replay never calls
    // the simulation or RNG, and each history owns its own command/fade cursor.
    unsafe fn update_history(&mut self, history: &mut History, scene: &Scene) -> Result<()> {
        assert!(Rc::ptr_eq(&self.gl, &history.gl));
        let size = history.size;
        let (view, projection) = geometry::camera(
            f32::from(u16::try_from(size[0])?) / f32::from(u16::try_from(size[1])?),
            scene.angle,
        );
        unsafe {
            self.prepare_meshes(history)?;
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, history.framebuffer);
            self.gl.viewport(0, 0, size[0], size[1]);
            self.gl.disable(glow::SCISSOR_TEST);
            self.gl.color_mask(true, true, true, true);
            if history.generation != Some(scene.generation) {
                // ponytail: bound synchronous replay of a starved shared fade;
                // chunk replay if >4096 passes becomes a real workload.
                if scene.fades > 4096 {
                    return Err("fade history exceeds synchronous replay budget".into());
                }
                self.gl.depth_mask(true);
                self.gl.clear_depth_f32(1.0);
                self.gl.clear_color(0.0, 0.0, 0.0, 1.0);
                self.gl
                    .clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                history.generation = Some(scene.generation);
                history.drawn = 0;
                history.faded = 0;
            }
            if scene.instances.len() > history.drawn {
                pipe_state(&self.gl);
                self.gl.use_program(Some(self.program));
                self.gl.active_texture(glow::TEXTURE0);
                self.gl.bind_texture(glow::TEXTURE_2D, self.texture);
                self.gl
                    .uniform_matrix_4_f32_slice(Some(&self.view), false, &view.to_cols_array());
                self.gl.uniform_matrix_4_f32_slice(
                    Some(&self.projection),
                    false,
                    &projection.to_cols_array(),
                );
                self.gl
                    .bind_buffer(glow::ARRAY_BUFFER, Some(self.instances));
                let bytes = bytemuck::cast_slice(&scene.instances);
                if bytes.len() > self.capacity {
                    self.capacity = (self.capacity * 2)
                        .max(bytes.len())
                        .max(64 * size_of::<Instance>());
                    self.gl.buffer_data_size(
                        glow::ARRAY_BUFFER,
                        i32::try_from(self.capacity)?,
                        glow::DYNAMIC_DRAW,
                    );
                }
                // The one shared upload buffer can contain another output's data.
                // Upload this scene, but draw ONLY commands beyond this cursor.
                self.gl
                    .buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
                for span in &scene.spans {
                    let start = span.start.max(history.drawn);
                    let end = span.start + span.count;
                    if start >= end {
                        continue;
                    }
                    let meshes = if self.texture.is_some() {
                        &history.meshes
                    } else {
                        &self.meshes
                    };
                    let mesh = &meshes[span.primitive as usize];
                    mesh.bind_instances(&self.gl, i32::try_from(start * size_of::<Instance>())?)?;
                    self.gl.draw_elements_instanced(
                        glow::TRIANGLES,
                        mesh.index_count,
                        glow::UNSIGNED_SHORT,
                        0,
                        i32::try_from(end - start)?,
                    );
                }
                history.drawn = scene.instances.len();
            }
            if scene.fades > history.faded {
                self.gl.disable(glow::DEPTH_TEST);
                self.gl.depth_mask(false);
                self.gl.disable(glow::CULL_FACE);
                self.gl.enable(glow::BLEND);
                self.gl.blend_equation(glow::FUNC_ADD);
                self.gl
                    .blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                self.gl.use_program(Some(self.fade_program));
                self.gl.bind_vertex_array(Some(self.fade_vao));
                for _ in history.faded..scene.fades {
                    self.gl.draw_arrays(glow::TRIANGLES, 0, 3);
                }
                history.faded = scene.fades;
            }
            let error = self.gl.get_error();
            if error != glow::NO_ERROR {
                return Err(format!("history update GL error {error:#x}").into());
            }
        }
        Ok(())
    }
}

impl GpuMesh {
    // SAFETY: the owning context is current. Only per-history meshes are retired;
    // the renderer's immutable solid meshes live until context teardown.
    unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vertices);
            gl.delete_buffer(self.indices);
        }
    }

    // SAFETY: owning context is current; vertices/indices are Pod and bounded.
    unsafe fn new(gl: &glow::Context, mesh: &Mesh) -> Result<Self> {
        unsafe {
            let vao = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(vao));
            let vertices = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertices));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&mesh.vertices),
                glow::STATIC_DRAW,
            );
            let indices = gl.create_buffer()?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(indices));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                bytemuck::cast_slice(&mesh.indices),
                glow::STATIC_DRAW,
            );
            for (location, offset) in [(0, 0), (1, 12)] {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(
                    location,
                    3,
                    glow::FLOAT,
                    false,
                    i32::try_from(size_of::<Vertex>())?,
                    offset,
                );
            }
            gl.enable_vertex_attrib_array(5);
            gl.vertex_attrib_pointer_f32(
                5,
                2,
                glow::FLOAT,
                false,
                i32::try_from(size_of::<Vertex>())?,
                24,
            );
            Ok(Self {
                vao,
                vertices,
                indices,
                index_count: i32::try_from(mesh.indices.len())?,
            })
        }
    }

    // SAFETY: shared instance buffer is bound and offsets/counts match its upload.
    unsafe fn bind_instances(&self, gl: &glow::Context, base: i32) -> Result<()> {
        let stride = i32::try_from(size_of::<Instance>())?;
        unsafe {
            gl.bind_vertex_array(Some(self.vao));
            for (location, components, offset) in [(2, 3, 0), (3, 4, 12)] {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(
                    location,
                    components,
                    glow::FLOAT,
                    false,
                    stride,
                    base + offset,
                );
                gl.vertex_attrib_divisor(location, 1);
            }
            gl.enable_vertex_attrib_array(4);
            gl.vertex_attrib_pointer_i32(4, 1, glow::UNSIGNED_INT, stride, base + 28);
            gl.vertex_attrib_divisor(4, 1);
        }
        Ok(())
    }
}

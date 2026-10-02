#version 300 es
precision highp float;

in vec3 v_diffuse;
in vec3 v_specular;
in vec2 v_uv;
flat in highp uint v_textured;
uniform highp sampler2D u_texture;
out vec4 frag_color;

void main() {
    vec4 texel = v_textured != 0u ? texture(u_texture, v_uv) : vec4(1.0);
    // Specular follows modulation. Alpha testing stays disabled, as in XP;
    // transparent texels still participate in its depth-writing/blending rules.
    frag_color = vec4(clamp(v_diffuse * texel.rgb + v_specular, 0.0, 1.0), texel.a);
}

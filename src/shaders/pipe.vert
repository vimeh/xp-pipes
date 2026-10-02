#version 300 es

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec3 a_center;
layout(location = 3) in vec4 a_rotation;
layout(location = 4) in uint a_material;
layout(location = 5) in vec2 a_uv;

uniform mat4 u_view;
uniform mat4 u_projection;
uniform vec3 u_ambient[28];
uniform vec3 u_diffuse[28];
uniform vec4 u_specular_power[28];

out vec3 v_diffuse;
out vec3 v_specular;
out vec2 v_uv;
flat out highp uint v_textured;

vec3 orient(vec3 v) {
    return v + 2.0 * cross(a_rotation.xyz, cross(a_rotation.xyz, v) + a_rotation.w * v);
}

void main() {
    vec4 position = u_view * vec4(orient(a_position) + a_center, 1.0);
    vec3 normal = normalize(mat3(u_view) * orient(a_normal));
    // The world light is (0,-50,-150). XP's fixed LH view negates X/Z;
    // illumination uses the opposite (vertex-to-light) direction in view space.
    vec3 light = normalize(vec3(0.0, 50.0, -150.0));
    float lambert = max(dot(normal, light), 0.0);
    v_textured = a_material >= 24u ? 1u : 0u;
    v_uv = a_uv;
    float ambient = v_textured != 0u ? (47.0 / 255.0 + 0.1) : 1.1;
    v_diffuse = clamp(u_ambient[a_material] * ambient + u_diffuse[a_material] * lambert, 0.0, 1.0);
    v_specular = vec3(0.0);
    if (lambert > 0.0 && v_textured == 0u) {
        // D3DRS_LOCALVIEWER defaults TRUE, unlike the old OpenGL remake.
        vec3 halfway = normalize(light + normalize(-position.xyz));
        vec4 material = u_specular_power[a_material];
        float highlight = pow(max(dot(normal, halfway), 0.0), material.w);
        v_specular = clamp(material.rgb * 0.6 * highlight, 0.0, 1.0);
    }
    gl_Position = u_projection * position;
}

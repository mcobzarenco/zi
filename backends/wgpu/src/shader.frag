#version 450

#extension GL_EXT_nonuniform_qualifier : require

#define PI 3.1415926535897932384626433832795
#define TAU 6.28318530718

#define TEX_INDEX_EMPTY_FLAG 1 << 13
#define TEX_INDEX_USE_TEXTURE_RGB_FLAG 1 << 14


layout(location=0) flat in vec3 background_colour;
layout(location=1) flat in vec3 foreground_colour;
layout(location=2) in vec2 tex_coords;
layout(location=3) nonuniformEXT flat in uint tex_index;

layout(location=0) out vec4 f_color;

layout(set = 0, binding = 0) uniform texture2D t_diffuse[2048];
layout(set = 0, binding = 1) uniform sampler s_diffuse;

// vec2 CRTCurveUV(vec2 uv){
//   uv = uv * 2.0 - 1.0;
//   vec2 offset = abs(uv.yx) / vec2( 1.0, 1.0 );
//   uv = uv + uv * offset * offset;
//   uv = uv * 0.5 + 0.5;
//   return uv;
// }

// float scanLineIntensity(float uv, float resolution, float opacity) {
//   uv = abs(sin(uv * resolution * TAU + PI));
//   // uv = (0.5 * uv) + 0.5;
//   // intensity = pow(intensity, opacity);

//   float roundness = 0.1;
//   float intensity = clamp(pow((uv * (1 - uv)) * resolution / roundness , opacity), 0.0, 1.0);
//   return intensity;
// }

// void maind() {
//   if (tex_index == 8192) {
//     f_color = vec4(background_colour, 1.0);
//   } else {

//     vec4 c = texture(sampler2D(t_diffuse[tex_index], s_diffuse), tex_coords);
//     f_color = c * vec4(foreground_colour, 1.0)  + (1.0 - c) * vec4(background_colour, 1.0);
//   }

//   f_color *= scanLineIntensity(tex_coords.x, 1, 0.2);
//   f_color *= scanLineIntensity(tex_coords.y, 1, 0.05);

//   // Apply gamma correction
//   float gamma = 2.2;
//   f_color.rgb = pow(f_color.rgb, vec3(gamma));
// }

void main() {
  bool is_empty = (tex_index & TEX_INDEX_EMPTY_FLAG) != 0;
  if (is_empty) {
    f_color = vec4(background_colour, 1.0);
  } else {
    vec4 tex_color = texture(sampler2D(t_diffuse[tex_index & 0xfff], s_diffuse), tex_coords);
    bool use_texture_rgb = (tex_index & TEX_INDEX_USE_TEXTURE_RGB_FLAG) != 0;
    if (use_texture_rgb) {
      f_color = tex_color.a * vec4(tex_color.rgb, 1.0)  + (1.0 - tex_color.a) * vec4(background_colour, 1.0);
    } else {
      f_color = tex_color.a * vec4(foreground_colour, 1.0)  + (1.0 - tex_color.a) * vec4(background_colour, 1.0);
    }
  }

  // Apply gamma correction
  float gamma = 2.2;
  f_color.rgb = pow(f_color.rgb, vec3(gamma));
}

// void main() {
//   f_color = vec4(background_colour, 1.0);

//   // Apply gamma correction
//   float gamma = 2.2;
//   f_color.rgb = pow(f_color.rgb, vec3(gamma));
// }

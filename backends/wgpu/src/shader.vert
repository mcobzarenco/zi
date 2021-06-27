#version 450

layout(location=0) in vec3 position;
layout(location=1) in vec3 background_colour;
layout(location=2) in vec3 foreground_colour;
layout(location=3) in vec2 tex_coords;
layout(location=4) in uint tex_index;

layout(location=0) flat out vec3 f_background_colour;
layout(location=1) flat out vec3 f_foreground_colour;
layout(location=2) out vec2 f_tex_coords;
layout(location=3) flat out uint f_tex_index;

void main() {
  f_background_colour = background_colour;
  f_foreground_colour = foreground_colour;
  f_tex_coords = tex_coords;
  f_tex_index = tex_index;

  gl_Position = vec4(position, 1.0);
}

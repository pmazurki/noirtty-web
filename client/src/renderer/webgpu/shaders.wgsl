// Terminal cell background shader

struct GridUniforms {
    canvas_size: vec2<f32>,
    cell_size: vec2<f32>,
    grid_size: vec2<f32>,
    _padding: vec2<f32>,
    selection_color: vec4<f32>,
    cursor_color: vec4<f32>,
    background_color: vec4<f32>,
}

@group(0) @binding(0)
var<uniform> uniforms: GridUniforms;

struct CellInstance {
    @location(0) pos: vec2<f32>,       // col, row
    @location(1) bg_color: vec4<f32>,  // background color
    @location(2) flags: u32,           // bit 0 = has_bg, bit 1 = is_cursor, bit 2 = is_selected, bit 3 = underline
    @location(3) fg_color: vec4<f32>,  // foreground color (for underline)
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local_pos: vec2<f32>,  // Position within cell (0-1)
    @location(2) flags: u32,
    @location(3) fg_color: vec4<f32>,
}

// Quad vertices (two triangles)
const QUAD_VERTICES: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(1.0, 1.0),
);

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: CellInstance,
) -> VertexOutput {
    var out: VertexOutput;

    // Get quad vertex position (0-1)
    let local_pos = QUAD_VERTICES[vertex_index];

    // Calculate pixel position
    let cell_origin = instance.pos * uniforms.cell_size;
    let pixel_pos = cell_origin + local_pos * uniforms.cell_size;

    // Convert to clip space (-1 to 1)
    let clip_x = (pixel_pos.x / uniforms.canvas_size.x) * 2.0 - 1.0;
    let clip_y = 1.0 - (pixel_pos.y / uniforms.canvas_size.y) * 2.0;

    out.position = vec4<f32>(clip_x, clip_y, 0.0, 1.0);
    out.color = instance.bg_color;
    out.local_pos = local_pos;
    out.flags = instance.flags;
    out.fg_color = instance.fg_color;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let has_bg = (in.flags & 1u) != 0u;
    let has_underline = (in.flags & 8u) != 0u;

    // Skip cells with no background and no underline
    if !has_bg && !has_underline {
        discard;
    }

    // Underline region at bottom of cell
    let in_underline_region = in.local_pos.y > 0.88 && in.local_pos.y < 0.96;

    // Draw underline on top of background
    if has_underline && in_underline_region {
        return in.fg_color;
    }

    // Draw background
    if has_bg {
        return in.color;
    }

    discard;
}

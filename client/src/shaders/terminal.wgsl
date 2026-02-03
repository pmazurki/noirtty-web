// Terminal cell rendering shader
//
// Uses instanced rendering to draw terminal cells efficiently.
// Each cell is a quad with background color and foreground glyph.

struct Uniforms {
    grid_size: vec2<f32>,
    cell_size: vec2<f32>,
    time: f32,
    _padding: vec3<f32>,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct VertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @location(0) position: vec2<f32>,      // Cell grid position (col, row)
    @location(1) fg_color: vec3<f32>,      // Foreground color
    @location(2) bg_color: vec3<f32>,      // Background color
    @location(3) glyph_id: u32,            // ASCII code of character
    @location(4) flags: u32,               // Flags: bold, italic, underline, cursor
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) fg_color: vec3<f32>,
    @location(1) bg_color: vec3<f32>,
    @location(2) uv: vec2<f32>,            // UV coordinates within cell
    @location(3) glyph_id: u32,
    @location(4) flags: u32,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;

    // Generate quad corners from vertex index (0-3 for triangle strip)
    let quad_x = f32(input.vertex_index & 1u);
    let quad_y = f32((input.vertex_index >> 1u) & 1u);

    // Calculate cell position in normalized device coordinates
    // NDC goes from -1 to 1, so we need to transform grid coords
    let cell_x = (input.position.x + quad_x) * uniforms.cell_size.x * 2.0 - 1.0;
    let cell_y = 1.0 - (input.position.y + quad_y) * uniforms.cell_size.y * 2.0;

    output.clip_position = vec4<f32>(cell_x, cell_y, 0.0, 1.0);
    output.fg_color = input.fg_color;
    output.bg_color = input.bg_color;
    output.uv = vec2<f32>(quad_x, quad_y);
    output.glyph_id = input.glyph_id;
    output.flags = input.flags;

    return output;
}

// Simple procedural font - renders basic ASCII glyphs using SDF-like approach
fn render_glyph(uv: vec2<f32>, glyph: u32) -> f32 {
    // Margin inside cell for glyph
    let margin = 0.15;
    let inner_uv = (uv - margin) / (1.0 - 2.0 * margin);

    // Skip non-printable characters (render as space)
    if glyph < 33u || glyph > 126u {
        return 0.0;
    }

    // Simple procedural rendering - creates rough approximation
    // In production, use SDF font atlas texture
    let px = inner_uv.x;
    let py = inner_uv.y;

    // Bounds check
    if px < 0.0 || px > 1.0 || py < 0.0 || py > 1.0 {
        return 0.0;
    }

    // Generate some visual pattern based on glyph ID
    // This is a placeholder - real implementation would sample SDF atlas
    let hash = f32(glyph * 127u + glyph * glyph) / 16384.0;
    let pattern = sin(px * 6.28 * (2.0 + hash)) * sin(py * 6.28 * (2.0 + hash * 0.5));

    // Create a simple block character representation
    let block_density = 0.4 + hash * 0.4;
    let in_block = f32(px > 0.1 && px < 0.9 && py > 0.1 && py < 0.85);

    return in_block * block_density;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let uv = input.uv;

    // Render background
    var color = input.bg_color;

    // Check cursor flag (bit 3)
    let is_cursor = (input.flags & 8u) != 0u;

    // Render glyph
    let glyph_alpha = render_glyph(uv, input.glyph_id);

    // Blend foreground with background
    if glyph_alpha > 0.01 {
        color = mix(color, input.fg_color, glyph_alpha);
    }

    // Draw cursor as a block
    if is_cursor {
        // Blink cursor using time
        let blink = (sin(uniforms.time * 6.28) + 1.0) * 0.5;
        if blink > 0.5 {
            // Invert colors for cursor
            color = vec3<f32>(1.0) - color;
        }
    }

    // Underline (bit 2)
    let is_underline = (input.flags & 4u) != 0u;
    if is_underline && uv.y > 0.9 {
        color = input.fg_color;
    }

    return vec4<f32>(color, 1.0);
}

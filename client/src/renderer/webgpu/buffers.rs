//! GPU buffer structures

use bytemuck::{Pod, Zeroable};

/// Instance data for a single terminal cell
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CellInstance {
    /// Cell position (col, row)
    pub pos: [f32; 2],
    /// Background color (RGBA normalized)
    pub bg_color: [f32; 4],
    /// Flags: bit 0 = has_bg, bit 1 = is_cursor, bit 2 = is_selected, bit 3 = underline
    pub flags: u32,
    /// Foreground color for underline (RGBA normalized)
    pub fg_color: [f32; 4],
}

impl CellInstance {
    /// Vertex buffer layout for instanced rendering
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CellInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // pos
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // bg_color
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                // flags
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Uint32,
                },
                // fg_color
                wgpu::VertexAttribute {
                    offset: 28,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

/// Uniform data for the grid
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GridUniforms {
    /// Canvas size in pixels
    pub canvas_size: [f32; 2],
    /// Cell size in pixels (physical)
    pub cell_size: [f32; 2],
    /// Grid size in cells
    pub grid_size: [f32; 2],
    /// Padding for alignment
    pub _padding: [f32; 2],
    /// Selection highlight color
    pub selection_color: [f32; 4],
    /// Cursor color
    pub cursor_color: [f32; 4],
    /// Background color
    pub background_color: [f32; 4],
}

impl Default for GridUniforms {
    fn default() -> Self {
        Self {
            canvas_size: [0.0, 0.0],
            cell_size: [0.0, 0.0],
            grid_size: [80.0, 24.0],
            _padding: [0.0, 0.0],
            selection_color: [0.15, 0.31, 0.47, 1.0],
            cursor_color: [0.75, 0.75, 0.75, 1.0],
            background_color: [0.118, 0.118, 0.118, 1.0],
        }
    }
}

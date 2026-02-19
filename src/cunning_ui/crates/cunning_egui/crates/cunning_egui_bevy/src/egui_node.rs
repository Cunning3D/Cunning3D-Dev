#![allow(unreachable_patterns)]
use crate::{
    render_systems::{
        EguiPipelines, EguiTextureBindGroups, EguiTextureId, EguiTransform, EguiTransforms,
        ExtractedEguiWindowUi,
    },
    EguiSettings,
};
use bytemuck::cast_slice;
use bevy::{
    asset::RenderAssetUsages,
    ecs::world::{FromWorld, World},
    prelude::{Entity, Handle, Resource, Shader},
    render::{
        render_resource::{
            BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType,
            BlendComponent, BlendFactor, BlendOperation, BlendState, Buffer, BufferAddress,
            BufferBindingType, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
            Extent3d, FragmentState, FrontFace, IndexFormat, LoadOp, MultisampleState, Operations,
            PipelineCache, PrimitiveState, RenderPassColorAttachment, RenderPassDescriptor,
            RenderPipelineDescriptor, SamplerBindingType, ShaderStages, ShaderType,
            SpecializedRenderPipeline, StoreOp, TextureDimension, TextureFormat, TextureSampleType,
            TextureViewDimension, VertexFormat, VertexState, VertexStepMode,
        },
        renderer::{RenderDevice, RenderQueue},
        view::ExtractedWindows,
    },
};
use bevy_image::{Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy_mesh::VertexBufferLayout;
use egui::{TextureFilter, TextureOptions};
use egui_wgpu::{Callback, CallbackResources, ScreenDescriptor, TargetFormat}; // Import Callback
use egui_wgpu::sdf::SdfFrameId;
use core::marker::PhantomData;
use std::sync::Mutex;

/// Egui shader.
pub const EGUI_SHADER_HANDLE: Handle<Shader> =
    Handle::Uuid(uuid::Uuid::from_u128(9898276442290979394), PhantomData);

/// Egui render pipeline.
#[derive(Resource)]
pub struct EguiPipeline {
    /// Transform bind group layout.
    pub transform_bind_group_layout: BindGroupLayout,
    /// Texture bind group layout.
    pub texture_bind_group_layout: BindGroupLayout,
    /// Transform bind group layout (descriptor for pipeline specialization).
    pub transform_bind_group_layout_desc: BindGroupLayoutDescriptor,
    /// Texture bind group layout (descriptor for pipeline specialization).
    pub texture_bind_group_layout_desc: BindGroupLayoutDescriptor,
}

impl FromWorld for EguiPipeline {
    fn from_world(render_world: &mut World) -> Self {
        let render_device = render_world.get_resource::<RenderDevice>().unwrap();

        let transform_entries = [BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: Some(EguiTransform::min_size()),
                },
                count: None,
            }];
        let transform_bind_group_layout_desc =
            BindGroupLayoutDescriptor::new("egui transform bind group layout", &transform_entries);
        let transform_bind_group_layout =
            render_device.create_bind_group_layout("egui transform bind group layout", &transform_entries);

        let texture_entries = [
            BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
        ];
        let texture_bind_group_layout_desc =
            BindGroupLayoutDescriptor::new("egui texture bind group layout", &texture_entries);
        let texture_bind_group_layout =
            render_device.create_bind_group_layout("egui texture bind group layout", &texture_entries);

        EguiPipeline {
            transform_bind_group_layout,
            texture_bind_group_layout,
            transform_bind_group_layout_desc,
            texture_bind_group_layout_desc,
        }
    }
}

/// Key for specialized pipeline.
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub struct EguiPipelineKey {
    /// Texture format of a window's swap chain to render to.
    pub texture_format: TextureFormat,
}

impl SpecializedRenderPipeline for EguiPipeline {
    type Key = EguiPipelineKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("egui render pipeline".into()),
            layout: vec![
                self.transform_bind_group_layout_desc.clone(),
                self.texture_bind_group_layout_desc.clone(),
            ],
            immediate_size: 0,
            vertex: VertexState {
                shader: EGUI_SHADER_HANDLE,
                shader_defs: Vec::new(),
                entry_point: Some("vs_main".into()),
                buffers: vec![VertexBufferLayout::from_vertex_formats(
                    VertexStepMode::Vertex,
                    [VertexFormat::Float32x2, VertexFormat::Float32x2, VertexFormat::Unorm8x4],
                )],
            },
            fragment: Some(FragmentState {
                shader: EGUI_SHADER_HANDLE,
                shader_defs: Vec::new(),
                entry_point: Some("fs_main".into()),
                targets: vec![Some(ColorTargetState {
                    format: key.texture_format,
                    blend: Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::OneMinusSrcAlpha,
                            operation: BlendOperation::Add,
                        },
                        alpha: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::OneMinusSrcAlpha,
                            operation: BlendOperation::Add,
                        },
                    }),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState {
                front_face: FrontFace::Cw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: MultisampleState::default(),
            zero_initialize_workgroup_memory: true,
        }
    }
}

enum DrawCommandVariant {
    Mesh {
        index_start: u32,
        index_count: u32,
        egui_texture: EguiTextureId,
    },
    Callback(egui::PaintCallback),
}

struct DrawCommand {
    clipping_zone: (u32, u32, u32, u32), // x, y, w, h
    variant: DrawCommandVariant,
}

/// Egui render node.
pub struct EguiNode {
    window_entity: Entity,
    vertex_data: Vec<u8>,
    vertex_buffer_capacity: usize,
    vertex_buffer: Option<Buffer>,
    index_data: Vec<u8>,
    index_buffer_capacity: usize,
    index_buffer: Option<Buffer>,
    draw_commands: Vec<DrawCommand>,
    callback_resources: Mutex<CallbackResources>,
    scale_factor: f32, // [NEW] Store scale factor
    frame_id: u64,
    #[cfg(debug_assertions)]
    last_holes_sig: u64,
}

impl EguiNode {
    /// Constructs Egui render node.
    pub fn new(window_entity: Entity) -> Self {
        EguiNode {
            window_entity,
            draw_commands: Vec::new(),
            vertex_data: Vec::new(),
            vertex_buffer_capacity: 0,
            vertex_buffer: None,
            index_data: Vec::new(),
            index_buffer_capacity: 0,
            index_buffer: None,
            callback_resources: Mutex::new(CallbackResources::new()),
            scale_factor: 1.0,
            frame_id: 0,
            #[cfg(debug_assertions)]
            last_holes_sig: u64::MAX, // force first log
        }
    }
}

impl EguiNode {
    pub fn update(&mut self, world: &mut World) {
        let mut extracted = world.resource_mut::<ExtractedEguiWindowUi>();
        let Some((window_size, render_output)) = extracted.0.get_mut(&self.window_entity) else {
            return;
        };
        let window_size = *window_size;
        let paint_jobs = std::mem::take(&mut render_output.paint_jobs);
        
        let egui_settings = &world.get_resource::<EguiSettings>().unwrap();

        let render_device = world.get_resource::<RenderDevice>().unwrap();

        let scale_factor = window_size.scale_factor * egui_settings.scale_factor;

        self.scale_factor = scale_factor; // Update scale factor

        if window_size.physical_width == 0.0 || window_size.physical_height == 0.0 {
            return;
        }

        let mut vertex_base: u32 = 0;
        let mut index_cursor: u32 = 0;
        self.frame_id = self.frame_id.wrapping_add(1);

        self.draw_commands.clear();
        self.vertex_data.clear();
        self.index_data.clear();

        let occ_res = world.get_resource::<crate::EguiOcclusionRects>();
        let (occ, occ_keys_sample) = occ_res
            .map(|o| {
                let keys = o.0.keys().take(4).copied().collect::<Vec<_>>();
                (o.0.get(&self.window_entity).cloned().unwrap_or_default(), keys)
            })
            .unwrap_or_default();

        fn overlap(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> bool {
            let (ax, ay, aw, ah) = a;
            let (bx, by, bw, bh) = b;
            let (ar, ab) = (ax.saturating_add(aw), ay.saturating_add(ah));
            let (br, bb) = (bx.saturating_add(bw), by.saturating_add(bh));
            ax < br && bx < ar && ay < bb && by < ab
        }

        fn subtract(zone: (u32, u32, u32, u32), hole: (u32, u32, u32, u32)) -> Vec<(u32, u32, u32, u32)> {
            if !overlap(zone, hole) { return vec![zone]; }
            let (zx, zy, zw, zh) = zone;
            let (hx, hy, hw, hh) = hole;
            let (zr, zb) = (zx + zw, zy + zh);
            let (hr, hb) = (hx + hw, hy + hh);
            let ix0 = zx.max(hx);
            let iy0 = zy.max(hy);
            let ix1 = zr.min(hr);
            let iy1 = zb.min(hb);
            if ix1 <= ix0 || iy1 <= iy0 { return vec![zone]; }
            let mut out = Vec::with_capacity(4);
            // top
            if iy0 > zy { out.push((zx, zy, zw, iy0 - zy)); }
            // bottom
            if iy1 < zb { out.push((zx, iy1, zw, zb - iy1)); }
            // left
            if ix0 > zx { out.push((zx, iy0, ix0 - zx, iy1 - iy0)); }
            // right
            if ix1 < zr { out.push((ix1, iy0, zr - ix1, iy1 - iy0)); }
            out.into_iter().filter(|z| z.2 > 0 && z.3 > 0).collect()
        }

        fn apply_holes(mut zones: Vec<(u32, u32, u32, u32)>, holes: &[(u32, u32, u32, u32)]) -> Vec<(u32, u32, u32, u32)> {
            for h in holes {
                zones = zones.into_iter().flat_map(|z| subtract(z, *h)).collect();
                if zones.is_empty() { break; }
            }
            zones
        }

        let holes_px: Vec<(u32, u32, u32, u32)> = occ
            .iter()
            .map(|r| {
                let x = (r.min.x * scale_factor).round().max(0.0) as u32;
                let y = (r.min.y * scale_factor).round().max(0.0) as u32;
                let w = (r.width() * scale_factor).round().max(0.0) as u32;
                let h = (r.height() * scale_factor).round().max(0.0) as u32;
                (x, y, w, h)
            })
            .filter(|z| z.2 > 0 && z.3 > 0)
            .collect();

        #[cfg(debug_assertions)]
        {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            holes_px.len().hash(&mut h);
            // hash first few rects to avoid large work
            for z in holes_px.iter().take(4) { z.hash(&mut h); }
            let sig = h.finish();
            if sig != self.last_holes_sig {
                self.last_holes_sig = sig;
                let (ww, hh) = (window_size.physical_width.max(1.0) as u64, window_size.physical_height.max(1.0) as u64);
                let max_hole_area = holes_px.iter().map(|z| z.2 as u64 * z.3 as u64).max().unwrap_or(0);
                bevy::log::warn!(
                    "EGUI_NODE_HOLES win={:?} win_key={} map_keys={:?} paint_jobs={} holes={} max_hole={:.1}% sf={:.3} first={:?}",
                    self.window_entity,
                    self.window_entity.index(),
                    occ_keys_sample,
                    paint_jobs.len(),
                    holes_px.len(),
                    (max_hole_area as f64) * 100.0 / ((ww * hh) as f64),
                    scale_factor,
                    holes_px.get(0).copied()
                );
            }
        }

        for egui::epaint::ClippedPrimitive {
            clip_rect,
            primitive,
        } in &paint_jobs
        {
            let (x, y, w, h) = (
                (clip_rect.min.x * scale_factor).round() as u32,
                (clip_rect.min.y * scale_factor).round() as u32,
                (clip_rect.width() * scale_factor).round() as u32,
                (clip_rect.height() * scale_factor).round() as u32,
            );

            if w < 1
                || h < 1
                || x >= window_size.physical_width as u32
                || y >= window_size.physical_height as u32
            {
                continue;
            }

            let x_viewport_clamp = (x + w).saturating_sub(window_size.physical_width as u32);
            let y_viewport_clamp = (y + h).saturating_sub(window_size.physical_height as u32);
            let clipping_zone = (
                x,
                y,
                w.saturating_sub(x_viewport_clamp).max(1),
                h.saturating_sub(y_viewport_clamp).max(1),
            );
            let zones = apply_holes(vec![clipping_zone], &holes_px);
            if zones.is_empty() { continue; }

            #[allow(unreachable_patterns)]
            match primitive {
                egui::epaint::Primitive::Mesh(mesh) => {
                    self.vertex_data
                        .extend_from_slice(cast_slice::<_, u8>(mesh.vertices.as_slice()));
                    let indices_with_offset = mesh
                        .indices
                        .iter()
                        .map(|i| i + vertex_base)
                        .collect::<Vec<_>>();
                    self.index_data
                        .extend_from_slice(cast_slice(indices_with_offset.as_slice()));
                    vertex_base += mesh.vertices.len() as u32;

                    let texture_handle = match mesh.texture_id {
                        egui::TextureId::Managed(id) => EguiTextureId::Managed(self.window_entity, id),
                        egui::TextureId::User(id) => EguiTextureId::User(id),
                    };

                    let index_start = index_cursor;
                    let index_count = mesh.indices.len() as u32;
                    index_cursor = index_cursor.wrapping_add(index_count);
                    for z in zones {
                        self.draw_commands.push(DrawCommand {
                            clipping_zone: z,
                            variant: DrawCommandVariant::Mesh { index_start, index_count, egui_texture: texture_handle.clone() },
                        });
                    }
                }
                egui::epaint::Primitive::Callback(callback) => {
                    for z in zones {
                        self.draw_commands.push(DrawCommand { clipping_zone: z, variant: DrawCommandVariant::Callback(callback.clone()) });
                    }
                }
            }
        }

        if self.vertex_data.len() > self.vertex_buffer_capacity {
            self.vertex_buffer_capacity = if self.vertex_data.len().is_power_of_two() {
                self.vertex_data.len()
            } else {
                self.vertex_data.len().next_power_of_two()
            };
            self.vertex_buffer = Some(render_device.create_buffer(&BufferDescriptor {
                label: Some("egui vertex buffer"),
                size: self.vertex_buffer_capacity as BufferAddress,
                usage: BufferUsages::COPY_DST | BufferUsages::VERTEX,
                mapped_at_creation: false,
            }));
        }
        if self.index_data.len() > self.index_buffer_capacity {
            self.index_buffer_capacity = if self.index_data.len().is_power_of_two() {
                self.index_data.len()
            } else {
                self.index_data.len().next_power_of_two()
            };
            self.index_buffer = Some(render_device.create_buffer(&BufferDescriptor {
                label: Some("egui index buffer"),
                size: self.index_buffer_capacity as BufferAddress,
                usage: BufferUsages::COPY_DST | BufferUsages::INDEX,
                mapped_at_creation: false,
            }));
        }
    }

    pub fn render(&self, world: &World, encoder: &mut egui_wgpu::wgpu::CommandEncoder) {

        let egui_pipelines = &world.get_resource::<EguiPipelines>().unwrap().0;
        let pipeline_cache = world.get_resource::<PipelineCache>().unwrap();
        // let egui_settings = world.get_resource::<EguiSettings>().unwrap(); // Removed, use stored scale_factor

        let extracted_windows = &world.get_resource::<ExtractedWindows>().unwrap().windows;
        let extracted_window = if let Some(extracted_window) = extracted_windows.get(&self.window_entity) {
            extracted_window
        } else {
            return; // No window
        };
        
        let bevy_fmt = extracted_window
            .swap_chain_texture_format
            .unwrap_or(TextureFormat::Bgra8UnormSrgb);

        let swap_chain_texture_view = if let Some(swap_chain_texture_view) = extracted_window.swap_chain_texture_view.as_ref() {
            swap_chain_texture_view
        } else {
            return; // No swapchain texture
        };

        let render_queue = world.get_resource::<RenderQueue>().unwrap();
        let render_device = world.get_resource::<RenderDevice>().unwrap();

        // --- Prepare Callbacks ---
        let scale_factor = self.scale_factor; // Use stored scale factor
        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [extracted_window.physical_width, extracted_window.physical_height],
            pixels_per_point: scale_factor,
        };

        let mut callback_resources = self.callback_resources.lock().unwrap();
        *callback_resources.entry::<SdfFrameId>().or_insert(SdfFrameId(self.frame_id)) =
            SdfFrameId(self.frame_id);
        let t = world
            .get_resource::<bevy::prelude::Time>()
            .map(|t| t.elapsed_secs())
            .unwrap_or(0.0);
        *callback_resources
            .entry::<egui_wgpu::sdf::SdfTime>()
            .or_insert(egui_wgpu::sdf::SdfTime(t)) = egui_wgpu::sdf::SdfTime(t);
        // Critical: SDF pipelines must match the active render target format (sRGB vs non-sRGB).
        // Bevy stores the surface format in `swap_chain_texture_format` (Option<TextureFormat>).
        let egui_fmt = match bevy_fmt {
            TextureFormat::Bgra8Unorm => egui_wgpu::wgpu::TextureFormat::Bgra8Unorm,
            TextureFormat::Bgra8UnormSrgb => egui_wgpu::wgpu::TextureFormat::Bgra8UnormSrgb,
            TextureFormat::Rgba8Unorm => egui_wgpu::wgpu::TextureFormat::Rgba8Unorm,
            TextureFormat::Rgba8UnormSrgb => egui_wgpu::wgpu::TextureFormat::Rgba8UnormSrgb,
            _ => egui_wgpu::wgpu::TextureFormat::Bgra8UnormSrgb,
        };
        *callback_resources.entry::<TargetFormat>().or_insert(TargetFormat(egui_fmt)) = TargetFormat(egui_fmt);

        for draw_command in &self.draw_commands {
            if let DrawCommandVariant::Callback(paint_callback) = &draw_command.variant {
                // Downcast to egui_wgpu::Callback which exposes .0 as Box<dyn CallbackTrait>
                if let Some(callback) = paint_callback.callback.downcast_ref::<Callback>() {
                    let device: &egui_wgpu::wgpu::Device = render_device.wgpu_device();
                    let queue: &egui_wgpu::wgpu::Queue = &**render_queue;
                    let buffers = callback.0.prepare(device, queue, &screen_descriptor, encoder, &mut callback_resources);
                    render_queue.submit(buffers);
                }
            }
        }

        // --- Finish Prepare Callbacks (required for batched SDF pipelines) ---
        for draw_command in &self.draw_commands {
            if let DrawCommandVariant::Callback(paint_callback) = &draw_command.variant {
                if let Some(callback) = paint_callback.callback.downcast_ref::<Callback>() {
                    let device: &egui_wgpu::wgpu::Device = render_device.wgpu_device();
                    let queue: &egui_wgpu::wgpu::Queue = &**render_queue;
                    let buffers = callback.0.finish_prepare(device, queue, encoder, &mut callback_resources);
                    render_queue.submit(buffers);
                }
            }
        }
        
        // ---

        let (vertex_buffer, index_buffer) = match (&self.vertex_buffer, &self.index_buffer) {
            (Some(vertex), Some(index)) => (vertex, index),
            _ => return,
        };

        render_queue.write_buffer(vertex_buffer, 0, &self.vertex_data);
        render_queue.write_buffer(index_buffer, 0, &self.index_data);

        let bind_groups = &world.get_resource::<EguiTextureBindGroups>().unwrap();

        let egui_transforms = world.get_resource::<EguiTransforms>().unwrap();

        let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("egui render pass"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: swap_chain_texture_view,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Load,
                            store: StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
        // Explicitly set full-window viewport; otherwise we may inherit a smaller viewport from earlier passes.
        render_pass.set_viewport(
            0.0,
            0.0,
            extracted_window.physical_width as f32,
            extracted_window.physical_height as f32,
            0.0,
            1.0,
        );
        render_pass.set_scissor_rect(0, 0, extracted_window.physical_width, extracted_window.physical_height);

        let Some(pipeline_id) = egui_pipelines.get(&extracted_window.entity) else { return; };
        let Some(pipeline) = pipeline_cache.get_render_pipeline(*pipeline_id) else { return; };

        render_pass.set_pipeline(pipeline);
        render_pass.set_vertex_buffer(0, *self.vertex_buffer.as_ref().unwrap().slice(..));
        render_pass.set_index_buffer(
            *self.index_buffer.as_ref().unwrap().slice(..),
            IndexFormat::Uint32,
        );

        let transform_buffer_offset = egui_transforms.offsets[&self.window_entity];
        let transform_buffer_bind_group = &egui_transforms.bind_group.as_ref().unwrap().1;
        render_pass.set_bind_group(0, transform_buffer_bind_group, &[transform_buffer_offset]);

        for draw_command in &self.draw_commands {
            if draw_command.clipping_zone.0 < extracted_window.physical_width
                && draw_command.clipping_zone.1 < extracted_window.physical_height
            {
                // Apply Scissor
                render_pass.set_scissor_rect(
                    draw_command.clipping_zone.0,
                    draw_command.clipping_zone.1,
                    draw_command.clipping_zone.2.min(
                        extracted_window
                            .physical_width
                            .saturating_sub(draw_command.clipping_zone.0),
                    ),
                    draw_command.clipping_zone.3.min(
                        extracted_window
                            .physical_height
                            .saturating_sub(draw_command.clipping_zone.1),
                    ),
                );

                match &draw_command.variant {
                    DrawCommandVariant::Mesh { index_start, index_count, egui_texture } => {
                        let texture_bind_group = match bind_groups.get(egui_texture) {
                            Some(texture_resource) => texture_resource,
                            None => {
                                continue;
                            }
                        };

                        render_pass.set_bind_group(1, texture_bind_group, &[]);

                        render_pass.draw_indexed(
                            *index_start..(*index_start + *index_count),
                            0,
                            0..1,
                        );
                    }
                    DrawCommandVariant::Callback(paint_callback) => {
                        if let Some(callback) = paint_callback.callback.downcast_ref::<Callback>() {
                            // `egui::PaintCallbackInfo` expects all rects in points, not pixels.
                            // `draw_command.clipping_zone` is in physical pixels, so convert back using `scale_factor`.
                            let inv_ppp = if scale_factor > 0.0 { 1.0 / scale_factor } else { 1.0 };
                            let info = egui::PaintCallbackInfo {
                                viewport: paint_callback.rect,
                                clip_rect: egui::Rect::from_min_size(
                                    egui::pos2(
                                        draw_command.clipping_zone.0 as f32 * inv_ppp,
                                        draw_command.clipping_zone.1 as f32 * inv_ppp,
                                    ),
                                    egui::vec2(
                                        draw_command.clipping_zone.2 as f32 * inv_ppp,
                                        draw_command.clipping_zone.3 as f32 * inv_ppp,
                                    ),
                                ), // Approximate clip rect from zone
                                pixels_per_point: scale_factor,
                                screen_size_px: [extracted_window.physical_width, extracted_window.physical_height],
                            };
                            
                            callback.0.paint(info, &mut render_pass, &callback_resources);

                            // IMPORTANT: callbacks are free to change pipeline/bindings.
                            // Restore egui pipeline + core bindings so subsequent meshes (selection/lines/text/etc) render correctly.
                            render_pass.set_pipeline(pipeline);
                            render_pass.set_vertex_buffer(0, *self.vertex_buffer.as_ref().unwrap().slice(..));
                            render_pass.set_index_buffer(
                                *self.index_buffer.as_ref().unwrap().slice(..),
                                IndexFormat::Uint32,
                            );
                            render_pass.set_bind_group(0, transform_buffer_bind_group, &[transform_buffer_offset]);
                            // Also restore scissor to this draw command's clipping zone,
                            // since callbacks may have tightened it.
                            // Also restore viewport: callbacks may set a smaller viewport and cause egui to flicker.
                            render_pass.set_viewport(0.0, 0.0, extracted_window.physical_width as f32, extracted_window.physical_height as f32, 0.0, 1.0);
                            render_pass.set_scissor_rect(
                                draw_command.clipping_zone.0,
                                draw_command.clipping_zone.1,
                                draw_command.clipping_zone.2.min(
                                    extracted_window
                                        .physical_width
                                        .saturating_sub(draw_command.clipping_zone.0),
                                ),
                                draw_command.clipping_zone.3.min(
                                    extracted_window
                                        .physical_height
                                        .saturating_sub(draw_command.clipping_zone.1),
                                ),
                            );
                        }
                    }
                }
            }
        }

    }
}

pub(crate) fn as_color_image(image: egui::ImageData) -> egui::ColorImage {
    match image {
        egui::ImageData::Color(image) => (*image).clone(),
    }
}

pub(crate) fn color_image_as_bevy_image(
    egui_image: &egui::ColorImage,
    sampler_descriptor: ImageSampler,
) -> Image {
    let pixels = egui_image
        .pixels
        .iter()
        // We unmultiply Egui textures to premultiply them later in the fragment shader.
        // As user textures loaded as Bevy assets are not premultiplied (and there seems to be no
        // convenient way to convert them to premultiplied ones), we do the this with Egui ones.
        .flat_map(|color| color.to_srgba_unmultiplied())
        .collect();

    Image {
        sampler: sampler_descriptor,
        ..Image::new(
            Extent3d {
                width: egui_image.width() as u32,
                height: egui_image.height() as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        )
    }
}

pub(crate) fn texture_options_as_sampler_descriptor(
    options: &TextureOptions,
) -> ImageSamplerDescriptor {
    fn convert_filter(filter: &TextureFilter) -> ImageFilterMode {
        match filter {
            egui::TextureFilter::Nearest => ImageFilterMode::Nearest,
            egui::TextureFilter::Linear => ImageFilterMode::Linear,
        }
    }
    let address_mode = match options.wrap_mode {
        egui::TextureWrapMode::ClampToEdge => ImageAddressMode::ClampToEdge,
        egui::TextureWrapMode::Repeat => ImageAddressMode::Repeat,
        egui::TextureWrapMode::MirroredRepeat => ImageAddressMode::MirrorRepeat,
    };
    ImageSamplerDescriptor {
        mag_filter: convert_filter(&options.magnification),
        min_filter: convert_filter(&options.minification),
        address_mode_u: address_mode,
        address_mode_v: address_mode,
        ..Default::default()
    }
}

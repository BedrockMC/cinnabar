//! A sandboxed mod pass and mod world primitives through their production entry points.
use crate::{gpu_snapshot, shader_source};

use bevy::math::{Mat4, Vec3};
use gpu_snapshot::{Draw, Gpu, SNAPSHOT_SIDE};
use mod_render::{Billboard, BillboardPattern, Decal, DecalStyle, Primitives};
use wgpu::util::DeviceExt;

const SIDE: usize = SNAPSHOT_SIDE as usize;

fn pixel(pixels: &[u8], u: f32, v: f32) -> [u8; 4] {
    let x = ((u * SIDE as f32) as usize).min(SIDE - 1);
    let y = ((v * SIDE as f32) as usize).min(SIDE - 1);
    let i = (y * SIDE + x) * 4;
    pixels[i..i + 4].try_into().unwrap()
}

/// A scene that is pure red on the left and pure blue on the right.
fn scene_texture(gpu: &Gpu) -> wgpu::TextureView {
    let mut texels = Vec::with_capacity(SIDE * SIDE * 4);
    for _ in 0..SIDE {
        for x in 0..SIDE {
            texels.extend(if x < SIDE / 2 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            });
        }
    }
    gpu.device
        .create_texture_with_data(
            &gpu.queue,
            &wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SNAPSHOT_SIDE,
                    height: SNAPSHOT_SIDE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &texels,
        )
        .create_view(&Default::default())
}

fn frame_words(params: &[f32]) -> Vec<f32> {
    let mut words = Mat4::IDENTITY.to_cols_array().to_vec();
    words.extend(Mat4::IDENTITY.to_cols_array());
    words.extend([0.0, 0.0, 0.0, 1.0]);
    let side = SNAPSHOT_SIDE as f32;
    words.extend([side, side, 1.0 / side, 1.0 / side]);
    words.extend([0.0; 4]);
    let mut slots = [0.0; 16];
    slots[..params.len()].copy_from_slice(params);
    words.extend(slots);
    words
}

#[test]
fn sandboxed_pass_grades_the_scene_from_its_uniform_params() {
    let Some(gpu) = Gpu::for_fixture("sandboxed_pass_grades_the_scene_from_its_uniform_params")
    else {
        return;
    };
    // Desaturates by param 0, then darkens towards the edge by param 1.
    let source = "fn effect(uv: vec2<f32>) -> vec3<f32> {
        let c = scene(uv);
        let grey = mix(c, vec3<f32>(luminance(c)), param(0u));
        let edge = length(uv - vec2<f32>(0.5)) * 2.0;
        return grey * (1.0 - param(1u) * smoothstep(0.6, 1.4, edge));
    }";
    let shader = mod_render::shader::compose(source, false).unwrap();
    let scene = scene_texture(&gpu);
    let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let raster = |params: &[f32]| {
        let frame = gpu.buffer(&frame_words(params), wgpu::BufferUsages::UNIFORM);
        let bindings = [
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&scene),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ];
        gpu.render(
            &shader,
            mod_render::shader::VERTEX_ENTRY,
            &[Draw {
                fragment: mod_render::shader::FRAGMENT_ENTRY,
                vertices: 0..3,
                bindings: &bindings,
                blend: None,
                write_depth: false,
            }],
        )
    };
    let untouched = raster(&[0.0, 0.0]);
    assert_eq!(pixel(&untouched, 0.25, 0.5), [255, 0, 0, 255]);
    assert_eq!(pixel(&untouched, 0.75, 0.5), [0, 0, 255, 255]);
    let grey = raster(&[1.0, 0.0]);
    let left = pixel(&grey, 0.25, 0.5);
    assert!(left[0] == left[1] && left[1] == left[2], "{left:?}");
    assert!(
        (50..58).contains(&left[0]),
        "red luminance is 0.2126: {left:?}"
    );
    let right = pixel(&grey, 0.75, 0.5);
    assert!(
        (16..21).contains(&right[0]),
        "blue luminance is 0.0722: {right:?}"
    );
    let vignette = raster(&[0.0, 1.0]);
    assert!(pixel(&vignette, 0.01, 0.01)[0] < 40, "corners darken");
    assert_eq!(pixel(&vignette, 0.4, 0.5), [255, 0, 0, 255], "centre stays");
    gpu_snapshot::save("mod-pass-grey", &grey);
    gpu_snapshot::save("mod-pass-vignette", &vignette);
}

/// The standalone View layout with a real world-from-view basis for billboards.
fn view_words(clip_from_view: Mat4, world_from_view: Mat4) -> Vec<f32> {
    let view_from_world = world_from_view.inverse();
    let clip_from_world = clip_from_view * view_from_world;
    let mut words = Vec::new();
    for matrix in [
        clip_from_world,
        clip_from_world,
        view_from_world,
        world_from_view,
        clip_from_view,
        clip_from_view.inverse(),
    ] {
        words.extend(matrix.to_cols_array());
    }
    let eye = world_from_view.w_axis;
    words.extend([eye.x, eye.y, eye.z, 1.0, 0.0, 0.0]);
    words.extend([SNAPSHOT_SIDE as f32, SNAPSHOT_SIDE as f32]);
    words
}

#[test]
fn decals_render_on_the_ground_and_respect_scene_depth() {
    let Some(gpu) = Gpu::for_fixture("decals_render_on_the_ground_and_respect_scene_depth") else {
        return;
    };
    let source =
        shader_source::standalone(include_str!("../../src/mod_render/primitives.wgsl"), &[]);
    // Looks straight down at the origin from ten blocks up; screen right is +X.
    let world_from_view =
        Mat4::look_at_rh(Vec3::new(0.0, 10.0, 0.0), Vec3::ZERO, Vec3::NEG_Z).inverse();
    let projection = Mat4::perspective_infinite_reverse_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1);
    let view = gpu.buffer(
        &view_words(projection, world_from_view),
        wgpu::BufferUsages::UNIFORM,
    );
    let frame = gpu.buffer(&[0.0; 4], wgpu::BufferUsages::UNIFORM);
    let occluder = Billboard {
        position: [-5.0, 5.0, 0.0],
        width: 10.0,
        height: 30.0,
        color: [0.0, 1.0, 0.0, 1.0],
        pattern: BillboardPattern::Solid,
        upright: false,
    };
    let decal = Decal {
        center: [0.0, 0.0, 0.0],
        radius: 8.0,
        color: [1.0, 0.1, 0.0, 0.9],
        progress: 0.75,
        style: DecalStyle::Telegraph,
    };
    let vertices = mod_render::geometry::build(&Primitives {
        decals: vec![decal],
        billboards: vec![occluder],
        ..Default::default()
    });
    let storage = gpu.buffer(bytemuck::cast_slice(&vertices), wgpu::BufferUsages::STORAGE);
    let bindings = [
        wgpu::BindGroupEntry {
            binding: 0,
            resource: view.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: storage.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: frame.as_entire_binding(),
        },
    ];
    let draw = |range, write_depth, blend| Draw {
        fragment: "mod_primitive_fragment",
        vertices: range,
        bindings: &bindings,
        blend,
        write_depth,
    };
    let premultiplied = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
    let open = gpu.render(
        &source,
        "mod_primitive_vertex",
        &[draw(0..6, false, premultiplied)],
    );
    let occluded = gpu.render(
        &source,
        "mod_primitive_vertex",
        &[draw(6..12, true, None), draw(0..6, false, premultiplied)],
    );
    let background = [31, 46, 64];
    let outside = pixel(&open, 0.02, 0.5);
    assert_eq!(&outside[..3], &background, "beyond the radius stays clear");
    let left = pixel(&open, 0.3, 0.5);
    let right = pixel(&open, 0.7, 0.5);
    assert!(
        left[0] > background[0] + 30 && left[2] < background[2],
        "{left:?}"
    );
    assert_eq!(left, right, "the fill is radially symmetric");
    let rim = pixel(&open, 0.5 + 0.97 * 0.4, 0.5);
    assert!(
        rim[0] > left[0],
        "the rim outshines the fill: {rim:?} {left:?}"
    );
    assert_eq!(
        pixel(&occluded, 0.3, 0.5),
        [0, 255, 0, 255],
        "nearer geometry hides decals"
    );
    assert_eq!(pixel(&occluded, 0.7, 0.5), right);
    gpu_snapshot::save("mod-telegraph-decal", &open);
    gpu_snapshot::save("mod-telegraph-decal-occluded", &occluded);
}

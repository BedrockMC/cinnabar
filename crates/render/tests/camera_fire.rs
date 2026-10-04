#[path = "support/gpu_snapshot.rs"]
mod gpu_snapshot;

use gpu_snapshot::{Draw, Gpu, SNAPSHOT_SIDE};
use wgpu::util::DeviceExt;

/// Original synthetic pixels isolate native cube orientation and point sampling.
#[test]
fn camera_fire_native_cube_is_open_above_and_keeps_pixel_edges_below() {
    let Some(gpu) = fixture_gpu() else {
        eprintln!("skipping camera_fire_native_cube: missing native GPU adapter fixture");
        return;
    };
    let texture = gpu.device.create_texture_with_data(
        &gpu.queue,
        &wgpu::TextureDescriptor {
            label: Some("original fire test pixels"),
            size: wgpu::Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 2,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &(0..2)
            .flat_map(|frame| {
                (0..4).flat_map(move |y| {
                    (0..4).flat_map(move |_| {
                        if y < 2 {
                            [0; 4]
                        } else if frame == 0 {
                            [200, 70, 10, 255]
                        } else {
                            [10, 70, 200, 255]
                        }
                    })
                })
            })
            .collect::<Vec<_>>(),
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    let render = |frame: f32| {
        let mut words = vec![0.0; 12 + render::MAX_SCREEN_OVERLAY_LAYERS * 8];
        words[0] = 1.0;
        words[4..8].copy_from_slice(&[frame, frame, 0.0, 1.0]);
        words[8..12].copy_from_slice(&[1.25, 0.7, 0.0, 0.0]);
        words[12..20].copy_from_slice(&[
            1.0,
            1.0,
            1.0,
            0.9,
            render::ScreenOverlayKind::Fire as u32 as f32,
            0.0,
            0.0,
            0.0,
        ]);
        let uniform = gpu.buffer(&words, wgpu::BufferUsages::UNIFORM);
        let bindings = [
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ];
        gpu.render(
            include_str!("../src/screen_overlay.wgsl"),
            "overlay_vertex",
            &[Draw {
                fragment: "overlay_fragment",
                vertices: 0..3,
                bindings: &bindings,
                blend: None,
                write_depth: false,
            }],
        )
    };
    let pixel = |pixels: &[u8], x: u32, y: u32| {
        let start = ((y * SNAPSHOT_SIDE + x) * 4) as usize;
        <[u8; 4]>::try_from(&pixels[start..start + 4]).unwrap()
    };
    for (frame, rgb) in [(0.0, [200, 70, 10]), (1.0, [10, 70, 200])] {
        let pixels = render(frame);
        assert_eq!(pixel(&pixels, SNAPSHOT_SIDE / 2, 0), [0; 4]);
        assert_eq!(pixel(&pixels, 0, 0), [0; 4]);
        let bottom = pixel(&pixels, SNAPSHOT_SIDE / 2, SNAPSHOT_SIDE - 1);
        assert_eq!(bottom[..3], rgb);
        assert!((i16::from(bottom[3]) - 230).abs() <= 1);
        let covered = pixels.chunks_exact(4).filter(|pixel| pixel[3] != 0).count();
        assert!(covered > 0 && covered < pixels.len() / 4);
        assert!(
            pixels
                .chunks_exact(4)
                .all(|pixel| pixel[3] == 0 || pixel[..3] == rgb)
        );
    }
}

fn fixture_gpu() -> Option<Gpu> {
    fn finish<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return value;
            }
            std::thread::yield_now();
        }
    }
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = finish(instance.request_adapter(&Default::default())).ok()?;
    if adapter.get_info().backend == wgpu::Backend::Noop {
        return None;
    }
    let (device, queue) = finish(adapter.request_device(&Default::default()))
        .expect("available camera-fire adapter must create the fixture device");
    Some(Gpu { device, queue })
}

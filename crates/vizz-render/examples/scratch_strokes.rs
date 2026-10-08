use vizz_render::camera::Camera;
use vizz_render::particles::{ParticleScene, Stroke, Uniforms};
use vizz_render::post::{PostChain, PostUniforms, SCENE_FORMAT};
use vizz_render::{GpuContext, output};
const W: u32 = 900; const H: u32 = 600;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = &args[1]; let shape_arg = args[2].clone();
    let stroke = Stroke::from_index(args[3].parse().unwrap());
    let len: f32 = args[4].parse().unwrap(); let count: u32 = args[5].parse().unwrap();
    let size: f32 = args.get(6).map(|s| s.parse().unwrap()).unwrap_or(0.004);
    let time: f32 = args.get(7).map(|s| s.parse().unwrap()).unwrap_or(1.0);
    let w: u32 = args.get(8).map(|s| s.parse().unwrap()).unwrap_or(W);
    let h: u32 = args.get(9).map(|s| s.parse().unwrap()).unwrap_or(H);
    let ctx = pollster::block_on(GpuContext::new(None)).unwrap();
    let mut scene = ParticleScene::new(&ctx, SCENE_FORMAT);
    let (shape, slot) = match shape_arg.parse::<f32>() { Ok(v) => (v, 0.0), Err(_) => {
        let slot = ParticleScene::loadable_slot(0).unwrap();
        let pts = vizz_render::generate::generate(&shape_arg).unwrap();
        scene.set_cloud(&ctx, slot, &pts, &shape_arg); (7.0, slot as f32) } };
    let target = output::OutputTarget::new(&ctx.device, w, h);
    let mut post = PostChain::new(&ctx, w * 2, h * 2, output::OUTPUT_FORMAT);
    let camera = Camera { orbit: 0.6, elevation: 0.3, distance: 3.2, aspect: w as f32 / h as f32, ..Default::default() };
    let cam = camera.uniforms();
    let u = Uniforms {
        view_proj: cam.view_proj, cam_right: cam.right, focus: camera.distance, cam_up: cam.up, defocus: 0.0,
        cam_position: cam.position, viewport_h: 0.0, time, aspect: camera.aspect, size, spread: 1.2,
        hue: 0.58, saturation: 0.8, brightness: 1.0, shape, morph: 0.0, twist: 0.0, palette: 1.0,
        color_spread: 0.8, color_drive: 3.0, cloud_a: slot, cloud_b: slot, cloud_morph: 0.0, room: Default::default(),
        lamp: Uniforms::UNLIT.lamp, lamp_tint: Uniforms::UNLIT.lamp_tint, light: Uniforms::UNLIT.light,
        sun_dir: Uniforms::UNLIT.sun_dir, sun_tint: Uniforms::UNLIT.sun_tint, stroke: stroke.lanes(len, 0.6),
        gravity: Default::default(), gravity_radius: Default::default(), gravity_amount: Default::default(),
        palette_rows: [4.0, 0.0, 0.0, 0.0], video: [0.0, 1.0, 0.0, 0.0],
    };
    let pu = PostUniforms { zoom: 1.0, glow: 0.15, aspect: camera.aspect, grade: 1.0, adapt: 1.0, max_gain: 64.0, ..Default::default() };
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    scene.render(&ctx, &mut enc, &post.scene_view, &u, count, true, wgpu::Color::BLACK);
    post.render(&ctx, &mut enc, &target.view, &pu);
    ctx.queue.submit([enc.finish()]);
    let padded = (w * 4).div_ceil(256) * 256;
    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (padded * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = ctx.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(target.texture.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None } }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
    ctx.queue.submit([enc.finish()]);
    let slice = buffer.slice(..); slice.map_async(wgpu::MapMode::Read, |_| {});
    ctx.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = slice.get_mapped_range().unwrap();
    let mut rgb = Vec::new();
    for row in 0..h as usize { for px in data[row * padded as usize..][..(w * 4) as usize].chunks(4) { rgb.extend_from_slice(&[px[2], px[1], px[0]]); } }
    image::RgbImage::from_raw(w, h, rgb).unwrap().save(out).unwrap();
}

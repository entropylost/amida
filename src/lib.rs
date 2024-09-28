#![feature(array_chunks)]

use std::{
    collections::HashMap,
    f32::consts::TAU,
    fs::File,
    path::{Path, PathBuf},
};

use cascade::{CascadeSettings, CascadeSize, RayLocation, RayLocationComps};
use color::{Diffuse, Opacity, Radiance};
use glam::Vec3 as FVec3;
use luisa::lang::types::vector::{Vec2, Vec3};
use radiance::RadianceCascades;
use sefirot::prelude::*;
use sefirot_testbed::{App, KeyCode, MouseButton};
use tiff::{
    decoder::{Decoder as TiffDecoder, DecodingResult},
    encoder::{colortype, TiffEncoder},
    tags::Tag,
    ColorType,
};

mod cascade;
mod color;
mod radiance;
mod trace;
mod utils;

struct World {
    size: [u32; 2],
    emissive: Tex2d<Radiance>,
    diffuse: Tex2d<Diffuse>,
    opacity: Tex2d<Opacity>,
    display_emissive: Tex2d<Radiance>,
    display_diffuse: Tex2d<Diffuse>,
    display_opacity: Tex2d<Opacity>,
}

const PAGENAME: Tag = Tag::Unknown(285);

impl World {
    fn new(width: u32, height: u32) -> Self {
        Self {
            size: [width, height],
            emissive: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
            diffuse: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
            opacity: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
            display_emissive: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
            display_diffuse: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
            display_opacity: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
        }
    }
    fn width(&self) -> u32 {
        self.size[0]
    }
    fn height(&self) -> u32 {
        self.size[1]
    }
    fn load(&self, path: impl AsRef<Path> + Copy) {
        let staging_buffer =
            DEVICE.create_buffer::<f32>(3 * (self.width() * self.height()) as usize);
        let staging_kernel = DEVICE.create_kernel::<fn(Tex2d<Radiance>)>(&track!(|texture| {
            let index = 3 * (dispatch_id().x + dispatch_id().y * self.width());
            let value = Vec3::expr(
                staging_buffer.read(index),
                staging_buffer.read(index + 1),
                staging_buffer.read(index + 2),
            );
            texture.write(dispatch_id().xy(), value);
        }));

        let file = File::open(path.as_ref().with_extension("tiff")).unwrap();
        let mut file = TiffDecoder::new(file).unwrap();

        let mut load = |name: &str, texture: &Tex2d<Radiance>| {
            assert_eq!(&file.get_tag_ascii_string(PAGENAME).unwrap(), name);
            assert_eq!(file.colortype().unwrap(), ColorType::RGB(32));
            assert_eq!(file.dimensions().unwrap(), (self.width(), self.height()));
            let image = file.read_image().unwrap();
            let DecodingResult::F32(image) = image else {
                unreachable!()
            };
            (
                staging_buffer.copy_from_async(&image),
                staging_kernel.dispatch_async([self.width(), self.height(), 1], texture),
            )
                .chain()
                .execute();
            if file.more_images() {
                file.next_image().unwrap();
            }
        };
        // Generally viewed in reverse.
        load("display_opacity", &self.display_opacity);
        load("display_diffuse", &self.display_diffuse);
        load("display_emissive", &self.display_emissive);
        load("opacity", &self.opacity);
        load("diffuse", &self.diffuse);
        load("emissive", &self.emissive);
    }
    fn save(&self, path: impl AsRef<Path> + Copy) {
        let staging_buffer =
            DEVICE.create_buffer::<f32>(3 * (self.width() * self.height()) as usize);
        let staging_kernel = DEVICE.create_kernel::<fn(Tex2d<Radiance>)>(&track!(|texture| {
            let index = 3 * (dispatch_id().x + dispatch_id().y * self.width());
            let value = texture.read(dispatch_id().xy());
            staging_buffer.write(index, value.x);
            staging_buffer.write(index + 1, value.y);
            staging_buffer.write(index + 2, value.z);
        }));

        let mut staging_host = vec![0.0_f32; 3 * (self.width() * self.height()) as usize];

        let file = File::create(path.as_ref()).unwrap();
        let mut file = TiffEncoder::new(file).unwrap();

        let mut save = |name: &str, texture: &Tex2d<Radiance>| {
            (
                staging_kernel.dispatch_async([self.width(), self.height(), 1], texture),
                staging_buffer.copy_to_async(&mut staging_host),
            )
                .chain()
                .execute();
            let mut image = file
                .new_image::<colortype::RGB32Float>(self.width(), self.height())
                .unwrap();
            image
                .encoder() // PageName
                .write_tag(PAGENAME, name)
                .unwrap();
            image.write_data(&staging_host).unwrap();
        };
        // Generally viewed in reverse.
        save("display_opacity", &self.display_opacity);
        save("display_diffuse", &self.display_diffuse);
        save("display_emissive", &self.display_emissive);
        save("opacity", &self.opacity);
        save("diffuse", &self.diffuse);
        save("emissive", &self.emissive);
    }
}

pub fn load_env(path: impl AsRef<Path> + Copy) -> Vec<FVec3> {
    let file = File::open(path.as_ref().with_extension("tiff")).unwrap();
    let mut file = TiffDecoder::new(file).unwrap();
    assert_eq!(file.colortype().unwrap(), ColorType::RGB(32));
    let image = file.read_image().unwrap();
    let DecodingResult::F32(image) = image else {
        unreachable!()
    };
    image
        .array_chunks::<3>()
        .copied()
        .map(FVec3::from)
        .collect::<Vec<_>>()
}
pub fn downsample_env(data: &[FVec3], buffer: &Buffer<Radiance>) {
    let ratio = data.len() / buffer.len();
    assert_eq!(data.len() % buffer.len(), 0);
    let staging = (0..buffer.len())
        .map(|i| {
            Vec3::from(
                data[i * ratio..(i + 1) * ratio]
                    .iter()
                    .copied()
                    .sum::<FVec3>()
                    / ratio as f32,
            )
        })
        .collect::<Vec<_>>();
    buffer.copy_from(&staging);
}
pub fn save_env(env: &[FVec3], path: impl AsRef<Path> + Copy) {
    let data = env
        .iter()
        .copied()
        .flat_map(<[f32; 3]>::from)
        .collect::<Vec<_>>();
    let width = 1 << (env.len().trailing_zeros() / 2);
    let file = File::create(path.as_ref()).unwrap();
    let mut file = TiffEncoder::new(file).unwrap();
    file.write_image::<colortype::RGB32Float>(width, env.len() as u32 / width, &data)
        .unwrap();
}

struct TraceWorld {
    size: [u32; 2],
    radiance: Tex2dView<Radiance>,
    opacity: Tex2dView<Opacity>,
    difference: Tex2dView<bool>,
    environment: BufferView<Radiance>,
}
impl TraceWorld {
    fn width(&self) -> u32 {
        self.size[0]
    }
    fn height(&self) -> u32 {
        self.size[1]
    }
}

pub fn main() {
    let grid_size = [512, 512];
    let grid_dispatch = [512, 512, 1];

    let app = App::new("Thelema Render", grid_size)
        .scale(4)
        // .dpi_override(2.0)
        .agx()
        .init();

    let bounce_cascades = CascadeSettings {
        base_interval: (1.5, 6.0),
        base_probe_spacing: 2.0,
        base_size: CascadeSize {
            probes: Vec2::new(256, 256),
            facings: 16, // 4 normally.
        },
        num_cascades: 5,
        spatial_factor: 1,
        angular_factor: 2,
    };

    let cascades = CascadeSettings {
        base_interval: (0.0, 1.0),
        base_probe_spacing: 1.0,
        base_size: CascadeSize {
            probes: Vec2::new(512, 512),
            facings: 4, // 4 normally.
        },
        num_cascades: 6,
        spatial_factor: 1,
        angular_factor: 2,
    };

    let world = World::new(grid_size[0], grid_size[1]);
    let environment =
        DEVICE.create_buffer(cascades.level_size(cascades.num_cascades).facings as usize);
    let bounce_environment = DEVICE.create_buffer(
        bounce_cascades
            .level_size(bounce_cascades.num_cascades)
            .facings as usize,
    );

    let radiance =
        DEVICE.create_tex2d::<Radiance>(PixelStorage::Float4, grid_size[0], grid_size[1], 1);
    let difference =
        DEVICE.create_tex2d::<bool>(PixelStorage::Float4, grid_size[0], grid_size[1], 1);

    let bounce_radiance_cascades = RadianceCascades::new(
        bounce_cascades,
        &TraceWorld {
            size: grid_size,
            radiance: radiance.view(0),
            opacity: world.opacity.view(0),
            difference: difference.view(0),
            environment: bounce_environment.view(..),
        },
    );
    let radiance_cascades = RadianceCascades::new(
        cascades,
        &TraceWorld {
            size: grid_size,
            radiance: radiance.view(0),
            opacity: world.display_opacity.view(0),
            difference: difference.view(0),
            environment: environment.view(..),
        },
    );

    let update_radiance_kernel = DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
        let total_radiance = Vec3::splat(0.0_f32).var();
        for i in 0_u32.expr()..bounce_cascades.facing_count(level) {
            let ray = RayLocation::from_comps_expr(RayLocationComps {
                probe: dispatch_id().xy() / bounce_cascades.probe_spacing(level).cast_u32(),
                facing: i,
                level,
            });
            *total_radiance += bounce_radiance_cascades.radiance.read(ray);
        }
        let avg_radiance = total_radiance / bounce_cascades.facing_count(level).cast_f32();

        let emissive = world.emissive.read(dispatch_id().xy());
        let diffuse = world.diffuse.read(dispatch_id().xy());

        radiance.write(dispatch_id().xy(), avg_radiance * diffuse + emissive);
    }));
    let finish_radiance_kernel = DEVICE.create_kernel::<fn(u32, bool)>(&track!(|level, raw| {
        let total_radiance = Vec3::splat(0.0_f32).var();
        for i in 0_u32.expr()..cascades.facing_count(level) {
            let ray = RayLocation::from_comps_expr(RayLocationComps {
                probe: dispatch_id().xy() / cascades.probe_spacing(level).cast_u32(),
                facing: i,
                level,
            });
            *total_radiance += radiance_cascades.radiance.read(ray);
        }
        let avg_radiance = total_radiance / cascades.facing_count(level).cast_f32();
        radiance.write(
            dispatch_id().xy(),
            if raw {
                avg_radiance
            } else {
                let emissive = world.display_emissive.read(dispatch_id().xy());
                let diffuse = world.display_diffuse.read(dispatch_id().xy());
                avg_radiance * diffuse + emissive
            },
        );
    }));

    let update_diff_kernel = DEVICE.create_kernel::<fn(Tex2d<Opacity>)>(&track!(|opacity| {
        let pos = dispatch_id().xy();
        let diff = false.var();
        let this_radiance = radiance.read(pos);
        let this_depth = opacity.read(pos);
        for i in 0_u32.expr()..4_u32.expr() {
            let offset = [
                Vec2::new(1, 0),
                Vec2::new(-1, 0),
                Vec2::new(0, 1),
                Vec2::new(0, -1),
            ]
            .expr()[i];
            let neighbor = pos.cast_i32() + offset;
            if (neighbor >= 0).all() && (neighbor < Vec2::from(grid_size).expr().cast_i32()).all() {
                let neighbor_radiance = radiance.read(neighbor.cast_u32());
                let neighbor_opacity = opacity.read(neighbor.cast_u32());
                if (neighbor_radiance != this_radiance).any()
                    || (neighbor_opacity != this_depth).any()
                {
                    *diff = true;
                    break;
                }
            }
        }
        difference.write(pos, **diff);
    }));

    let mut display_level = 0;

    let display_kernel = DEVICE.create_kernel::<fn(bool)>(&track!(|show_diff| {
        app.display().write(
            dispatch_id().xy(),
            radiance.read(dispatch_id().xy())
                + if show_diff {
                    difference.read(dispatch_id().xy()).cast_u32().cast_f32() * 5.0
                } else {
                    0.0_f32.expr()
                },
        );
    }));

    let mut merge_variant = 0;
    let mut num_bounces = 0;
    let mut run_final = true;
    let mut show_diff = false;
    let mut raw_radiance = true;

    let mut t = 0;

    let mut total_runtime = 0.0;

    let draw_kernel = DEVICE.create_kernel::<fn(Vec2<f32>, f32, Vec3<f32>, Vec3<f32>, Vec3<f32>)>(
        &track!(|pos, radius, emiss, diff, opacity| {
            if (dispatch_id().xy().cast_f32() - pos).length() < radius {
                world.emissive.write(dispatch_id().xy(), emiss);
                world.diffuse.write(dispatch_id().xy(), diff);
                world.opacity.write(dispatch_id().xy(), opacity);
                world.display_opacity.write(dispatch_id().xy(), opacity);
            }
        }),
    );

    #[rustfmt::skip]
    let material_map = vec![
        (MouseButton::Left, (Vec3::splat(0.0), Vec3::splat(1.0), Vec3::splat(1000.0))),
        (MouseButton::Middle, (Vec3::splat(0.0), Vec3::splat(0.0), Vec3::splat(0.0))),
        (MouseButton::Back, (Vec3::splat(0.0), Vec3::splat(0.0), Vec3::new(0.01, 0.1, 0.1))),
        (MouseButton::Right, (Vec3::splat(15.0), Vec3::splat(0.0), Vec3::splat(0.3))),
    ].into_iter().collect::<HashMap<_, _>>();

    let draw = |pos: Vec2<f32>, r: f32, x: MouseButton| {
        let (emiss, diff, opacity) = material_map[&x];
        draw_kernel.dispatch(grid_dispatch, &pos, &r, &emiss, &diff, &opacity);
    };

    let env_file_name = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "env/default.tiff".to_string());
    let world_file_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "world/default.tiff".to_string());

    if std::fs::exists(&env_file_name).unwrap_or(false) {
        let data = load_env(&env_file_name);
        downsample_env(&data, &environment);
        downsample_env(&data, &bounce_environment);
    }
    if std::fs::exists(&world_file_name).unwrap_or(false) {
        world.load(&world_file_name);
    }

    app.run(|rt, scope| {
        drop(scope);

        t += 1;

        for button in material_map.keys() {
            if rt.pressed_button(*button) {
                let pos = rt.cursor_position;
                draw(pos, 10.0, *button);
            }
        }

        if rt.just_pressed_key(KeyCode::Enter) {
            merge_variant = (merge_variant + 1) % radiance_cascades.merge_kernel_count();
        }
        if rt.just_pressed_key(KeyCode::KeyL) {
            display_level = (display_level + 1) % cascades.num_cascades;
            println!("Display level: {}", display_level);
        }
        if rt.just_pressed_key(KeyCode::KeyB) {
            num_bounces = (num_bounces + 1) % 4;
            println!("Num bounces: {}", num_bounces);
        }
        if rt.just_pressed_key(KeyCode::KeyF) {
            run_final = !run_final;
            println!("Run final: {}", run_final);
        }
        if rt.just_pressed_key(KeyCode::KeyD) {
            show_diff = !show_diff;
            println!("Show diff: {}", show_diff);
        }
        if rt.just_pressed_key(KeyCode::KeyR) {
            raw_radiance = !raw_radiance;
            println!("Raw radiance: {}", raw_radiance);
        }
        if rt.just_pressed_key(KeyCode::KeyS) {
            let mut path = PathBuf::from(&world_file_name);
            if !rt.pressed_key(KeyCode::ControlLeft) {
                let ext = path.extension().unwrap_or_default();
                let mut file_name = path.file_stem().unwrap().to_owned();
                file_name.push("-");
                file_name.push(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .expect("Temporal anomaly detected.")
                        .as_millis()
                        .to_string(),
                );
                file_name.push(".");
                file_name.push(ext);
                path.set_file_name(file_name);
            }
            world.save(&path);
            println!("Saved");
        }
        if rt.just_pressed_key(KeyCode::KeyL) {
            world.load(&world_file_name);
            println!("Loaded");
        }

        let before_time = std::time::Instant::now();

        (
            world
                .emissive
                .view(0)
                .copy_to_texture_async(&radiance.view(0)),
            (0..num_bounces)
                .map(|_i| {
                    (
                        update_diff_kernel
                            .dispatch_async(grid_dispatch, &world.opacity)
                            .debug("Update diff"),
                        bounce_radiance_cascades.update(0),
                        update_radiance_kernel
                            .dispatch_async(grid_dispatch, &0)
                            .debug("Update radiance"),
                    )
                        .chain()
                })
                .collect::<Vec<_>>(),
            run_final.then(|| {
                (
                    update_diff_kernel
                        .dispatch_async(grid_dispatch, &world.display_opacity)
                        .debug("Update diff"),
                    radiance_cascades.update(merge_variant),
                    finish_radiance_kernel
                        .dispatch_async(grid_dispatch, &display_level, &raw_radiance)
                        .debug("Finish radiance"),
                )
                    .chain()
            }),
            display_kernel
                .dispatch_async(grid_dispatch, &show_diff)
                .debug("Display"),
        )
            .chain()
            .execute();
        total_runtime += before_time.elapsed().as_secs_f32() * 1000.0;
        if t % 100 == 0 {
            println!("Frame time: {:?}ms", total_runtime / 100.0);
            total_runtime = 0.0;
        }
    });
}

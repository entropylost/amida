use super::*;
use cascade::*;
use trace::trace_radiance;
use utils::pcg3df;

mod bilinear_fix;
mod bilinear_fix_sep;
mod nearest;
mod single_stochastic;

pub struct RadianceCascades {
    settings: CascadeSettings,
    pub radiance: CascadeStorage<Vec3<f32>>,
    merge_kernels: Vec<luisa::runtime::Kernel<fn(u32)>>,
}

impl RadianceCascades {
    pub fn new(settings: CascadeSettings, world: &TraceWorld) -> Self {
        assert_eq!(settings.base_size.facings % settings.branches(), 0);
        let radiance = CascadeStorage::new(CascadeSettings {
            base_size: CascadeSize {
                facings: settings.base_size.facings / settings.branches(),
                ..settings.base_size
            },
            ..settings
        });

        let merge_kernels = vec![
            DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
                set_block_size([16, 8, 4]);
                bilinear_fix_sep::merge(world, settings, &radiance, level);
            })),
            DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
                set_block_size([16, 4, 2]);
                bilinear_fix_sep::merge(world, settings, &radiance, level);
            })),
            DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
                set_block_size([32, 2, 2]);
                bilinear_fix_sep::merge(world, settings, &radiance, level);
            })),
            //             DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
            //                 full_stochastic::merge(world, settings, &radiance, level);
            //             })),
            //             DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
            //                 bilinear_fix::merge(world, settings, &radiance, level);
            //             })),
            //             DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
            //                 nearest::merge(world, settings, &radiance, level);
            //             })),
        ];

        Self {
            settings,
            radiance,
            merge_kernels,
        }
    }
    pub fn merge_kernel_count(&self) -> usize {
        self.merge_kernels.len()
    }
    pub fn update(&self, _variant: usize) -> impl AsNodes {
        let mut commands = vec![];
        for level in (0..self.settings.num_cascades).rev() {
            let level_size = self.settings.level_size(level);
            commands.push(
                self.merge_kernels[(level / 2) as usize]
                    .dispatch_async(
                        [
                            level_size.facings * 4,
                            level_size.probes.x,
                            level_size.probes.y,
                        ],
                        &level,
                    )
                    .debug(format!("merge level {}", level)),
            );
        }
        commands.chain()
    }
}

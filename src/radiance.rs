use super::*;
use cascade::*;
use trace::trace_radiance;
use utils::pcg3df;

mod bilinear_fix;
mod full_stochastic;
mod nearest;
mod single_stochastic;

pub struct RadianceCascades {
    settings: CascadeSettings,
    pub radiance: CascadeStorage<Vec3<f32>>,
    merge_kernels: Vec<luisa::runtime::Kernel<fn(u32)>>,
}

impl RadianceCascades {
    pub fn new(settings: CascadeSettings, world: &World) -> Self {
        let radiance = CascadeStorage::new(settings);

        let merge_kernels = vec![
            DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
                single_stochastic::merge(world, settings, &radiance, level);
            })),
            DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
                full_stochastic::merge(world, settings, &radiance, level);
            })),
            DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
                bilinear_fix::merge(world, settings, &radiance, level);
            })),
            DEVICE.create_kernel::<fn(u32)>(&track!(|level| {
                nearest::merge(world, settings, &radiance, level);
            })),
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
    pub fn update(&self, variant: usize) -> impl AsNodes {
        let mut commands = vec![];
        for level in (0..self.settings.num_cascades).rev() {
            let level_size = self.settings.level_size(level);
            commands.push(
                self.merge_kernels[variant]
                    .dispatch_async(
                        [level_size.probes.x, level_size.probes.y, level_size.facings],
                        &level,
                    )
                    .debug(format!("merge level {}", level)),
            );
        }
        commands.chain()
    }
}

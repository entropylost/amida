// TODO: The algorithms seem to break on the upper left edges and causes nans.

use std::ops::Mul;

use super::*;
use cascade::*;
use trace::trace_radiance;
use utils::pcg3df;

mod bilinear_fix;
mod nearest;
mod single_stochastic;

fn zip3(a: [u32; 3], b: [u32; 3], f: impl Fn(u32, u32) -> u32) -> [u32; 3] {
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

struct MergeFunction {
    function: fn(&TraceWorld, CascadeSettings, &CascadeStorage<Radiance>, Expr<u32>),
    dispatch_scaling: [u32; 3],
    min_block_size: [u32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TuningSettings {
    pub block_sizes: HashMap<Vec<u32>, [u32; 3]>,
}
struct LoadedTuningSettings {
    block_sizes: Vec<[u32; 3]>,
    assignments: Vec<u32>,
}
impl TuningSettings {
    fn load(self, num_cascades: u32) -> LoadedTuningSettings {
        let mut assignments = vec![u32::MAX; num_cascades as usize];
        let mut block_sizes = Vec::new();
        for (levels, block_size) in self.block_sizes {
            let i = block_sizes.len() as u32;
            block_sizes.push(block_size);
            for level in levels {
                if level < num_cascades {
                    assignments[level as usize] = i;
                }
            }
        }
        assert!(
            assignments.iter().all(|&x| x != u32::MAX),
            "Missing block size assignment"
        );
        LoadedTuningSettings {
            block_sizes,
            assignments,
        }
    }
}

struct MergeKernel {
    kernels: Vec<luisa::runtime::Kernel<fn(u32)>>,
    dispatch_scaling: [u32; 3],
}

pub struct RadianceCascades {
    settings: CascadeSettings,
    pub radiance: CascadeStorage<Radiance>,
    tuning: LoadedTuningSettings,
    merge_kernels: Vec<MergeKernel>,
}

impl RadianceCascades {
    pub fn new(settings: CascadeSettings, world: &TraceWorld, tuning: TuningSettings) -> Self {
        assert_eq!(settings.base_size.facings % settings.branches(), 0, "The amount of facings must be divisible by the amount of branches for prefiltering to work");
        let radiance = CascadeStorage::new(CascadeSettings {
            base_size: CascadeSize {
                facings: settings.base_size.facings / settings.branches(),
                ..settings.base_size
            },
            ..settings
        });

        let tuning = tuning.load(settings.num_cascades);

        let merge_fns = vec![
            MergeFunction {
                function: single_stochastic::merge,
                dispatch_scaling: [1, 1, 1],
                min_block_size: [4, 1, 1],
            },
            MergeFunction {
                function: nearest::merge,
                dispatch_scaling: [1, 1, 1],
                min_block_size: [4, 1, 1],
            },
            MergeFunction {
                function: bilinear_fix::merge,
                dispatch_scaling: [4, 1, 1],
                min_block_size: [16, 1, 1],
            },
        ];

        let merge_kernels = merge_fns
            .into_iter()
            .map(|merge| MergeKernel {
                kernels: tuning
                    .block_sizes
                    .iter()
                    .map(|block_size| {
                        DEVICE.create_kernel_async::<fn(u32)>(&|level| {
                            set_block_size(zip3(*block_size, merge.min_block_size, Ord::max));
                            (merge.function)(world, settings, &radiance, level);
                        })
                    })
                    .collect::<Vec<_>>(),
                dispatch_scaling: merge.dispatch_scaling,
            })
            .collect::<Vec<_>>();

        Self {
            settings,
            radiance,
            tuning,
            merge_kernels,
        }
    }
    pub fn merge_kernel_count(&self) -> usize {
        self.merge_kernels.len()
    }
    pub fn update(&self, variant: usize) -> impl AsNodes {
        let mut commands = vec![];

        let merge = &self.merge_kernels[variant];

        for level in (0..self.settings.num_cascades).rev() {
            let level_size = self.settings.level_size(level);
            commands.push(
                merge.kernels[self.tuning.assignments[level as usize] as usize]
                    .dispatch_async(
                        zip3(
                            [level_size.facings, level_size.probes.x, level_size.probes.y],
                            merge.dispatch_scaling,
                            Mul::mul,
                        ),
                        &level,
                    )
                    .debug(format!("merge level {}", level)),
            );
        }
        commands.chain()
    }
}

use luisa::lang::{
    functions::{sync_block, thread_id},
    types::shared::Shared,
};

use super::*;

#[allow(unused)]
#[tracked]
pub fn merge(
    world: &TraceWorld,
    settings: CascadeSettings,
    radiance: &CascadeStorage<Radiance>,
    level: Expr<u32>,
) {
    let probe = dispatch_id().yz();
    let facing = dispatch_id().x;

    let probe_pos = settings.probe_location(probe, level);

    let ray_dir = settings.facing_direction(facing, level);

    let interval = settings.interval(level);

    let next_level = level + 1;
    let samples = settings.bilinear_samples(probe, next_level);

    let rand = pcg3df(dispatch_id() + Vec3::expr(0, 0, level << 16));
    let next_probe = samples.base_index + (rand.xy() < samples.fract).cast_u32();
    let next_probe_pos = settings.probe_location(next_probe, next_level);
    let ray_start = probe_pos + ray_dir * interval.x;
    let ray_end = next_probe_pos + ray_dir * interval.y;
    let ray_fluence = trace_radiance(
        world,
        ray_start,
        (ray_end - ray_start).normalize(),
        Vec2::expr(0.0, (ray_end - ray_start).length()),
    );

    let next_ray = RayLocation::from_comps_expr(RayLocationComps {
        probe: next_probe,
        facing,
        level: next_level,
    });

    let next_radiance = if next_level < settings.num_cascades {
        radiance.read(next_ray)
    } else {
        world.environment.read(facing)
    };

    let radiance_shared = Shared::<Radiance>::new(block_size().iter().product::<u32>() as usize);

    let probe_offset = block_size()[0] * (thread_id().y + block_size()[1] * thread_id().z);

    radiance_shared.write(
        thread_id().x + probe_offset,
        ray_fluence.over_color(next_radiance),
    );

    // Should be unnecessary since each warp includes 4 facings,
    // but we don't have a sync_warp function and this can apparently break.
    sync_block();

    if facing % settings.branches() == 0 {
        let total_radiance = (0..settings.branches())
            .map(|i| radiance_shared.read(thread_id().x + i + probe_offset))
            .reduce(AddExpr::add)
            .unwrap();
        let avg_radiance = total_radiance / settings.branches() as f32;
        radiance.write(
            RayLocation::from_comps_expr(RayLocationComps {
                probe,
                facing: facing / settings.branches(),
                level,
            }),
            avg_radiance,
        );
    }
}

use super::*;

/*
5, 4: [2, 2, 32],
3, 2: [4, 2, 16],
1, 0: [4, 8, 4],
*/

#[tracked]
pub fn merge(
    world: &TraceWorld,
    settings: CascadeSettings,
    radiance: &CascadeStorage<Radiance>,
    level: Expr<u32>,
) {
    let probe = dispatch_id().xy();
    let ray = RayLocation::from_comps_expr(RayLocationComps {
        probe,
        facing: dispatch_id().z,
        level,
    });

    let total_radiance = Radiance::splat(0.0_f32).var();
    for i in 0..settings.branches() {
        let facing = dispatch_id().z * settings.branches() + i;

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

        *total_radiance += ray_fluence.over_color(next_radiance);
    }
    let avg_radiance = total_radiance / settings.branches() as f32;
    radiance.write(ray, avg_radiance);
}

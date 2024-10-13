use amida::save_env;
use glam::Vec3 as FVec3;
use std::f32::consts::TAU;

fn skylight(angle: f32) -> FVec3 {
    // Default:
    let sky_color = FVec3::new(0.3, 0.7, 1.0);
    let sun_color = FVec3::new(1.0, 1.0, 0.8) * 5.0;
    // Night:
    // let sky_color = FVec3::new(0.2, 0.15, 0.6) * 0.3;
    // let sun_color = FVec3::new(1.0, 0.3, 0.1) * 0.0;
    // Desert:
    // let sky_color = FVec3::new(0.6, 0.7, 1.0);
    // let sun_color = FVec3::new(1.0, 0.9, 0.4) * 4.0;
    // Golden:
    // let sky_color = FVec3::new(0.6, 0.5, 0.7);
    // let sun_color = FVec3::new(1.0, 0.8, 0.2) * 4.0;

    let sun_size = 0.3;
    let sun_angle = 1.0;

    let sun_color = if (angle - sun_angle).abs() < sun_size {
        sun_color
    } else {
        FVec3::ZERO
    };
    sun_color + sky_color * angle.sin().max(0.0).powi(2)
}

fn main() {
    let env_facings = 4 << (2 * 8);
    let data = (0..env_facings)
        .map(|i| {
            let angle = TAU - i as f32 / env_facings as f32 * TAU;
            skylight(angle)
        })
        .collect::<Vec<_>>();
    save_env(&data, "env/default.tiff");
}

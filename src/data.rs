use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub world_size: [u32; 2],
    pub pixel_size: u32,
    pub dpi: f64,
    pub bounce_cascades: CascadeSettings,
    pub cascades: CascadeSettings,
    pub merge_variant: usize,
    pub num_bounces: usize,
    pub run_final: bool,
    pub show_diff: bool,
    pub raw_radiance: bool,
    pub display_level: u32,
    pub brush_radius: f32,
    pub mouse_materials: HashMap<MouseButton, Material>,
    pub key_materials: HashMap<KeyCode, Material>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            world_size: [512, 512],
            pixel_size: 2,
            dpi: 1.0,
            bounce_cascades: CascadeSettings {
                base_interval: (1.5, 6.0),
                base_probe_spacing: 2.0,
                base_size: CascadeSize {
                    probes: Vec2::new(256, 256),
                    facings: 16,
                },
                num_cascades: 5,
                spatial_factor: 1,
                angular_factor: 2,
            },
            cascades: CascadeSettings {
                base_interval: (0.0, 1.0),
                base_probe_spacing: 1.0,
                base_size: CascadeSize {
                    probes: Vec2::new(512, 512),
                    facings: 4,
                },
                num_cascades: 6,
                spatial_factor: 1,
                angular_factor: 2,
            },
            merge_variant: 0,
            num_bounces: 0,
            run_final: true,
            show_diff: false,
            raw_radiance: false,
            display_level: 0,
            brush_radius: 5.0,
            #[rustfmt::skip]
            mouse_materials: [
                (MouseButton::Left, Material::new(FVec3::splat(0.0), FVec3::splat(1.0), FVec3::splat(1000.0))),
                (MouseButton::Middle, Material::new(FVec3::splat(5.0), FVec3::splat(0.0), FVec3::splat(0.3))),
                (MouseButton::Right, Material::new(FVec3::splat(0.0), FVec3::splat(0.0), FVec3::splat(0.0))),
                (MouseButton::Back, Material::new(FVec3::splat(0.0), FVec3::splat(0.0), FVec3::new(0.1, 0.1, 0.01))),
                (MouseButton::Forward, Material::new(FVec3::splat(0.0), FVec3::splat(0.0), FVec3::new(0.01, 0.1, 0.1))),
            ].into_iter().collect(),
            key_materials: [].into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Material {
    pub emissive: FVec3,
    pub diffuse: FVec3,
    pub opacity: FVec3,
    pub display_emissive: FVec3,
    pub display_diffuse: FVec3,
    pub display_opacity: FVec3,
}
impl Default for Material {
    fn default() -> Self {
        Self {
            emissive: FVec3::splat(0.0),
            diffuse: FVec3::splat(1.0),
            opacity: FVec3::splat(0.0),
            display_emissive: FVec3::splat(0.0),
            display_diffuse: FVec3::splat(1.0),
            display_opacity: FVec3::splat(0.0),
        }
    }
}
impl Material {
    fn new(emissive: FVec3, diffuse: FVec3, opacity: FVec3) -> Self {
        Self {
            emissive,
            diffuse,
            opacity,
            display_emissive: FVec3::ZERO,
            display_diffuse: FVec3::splat(1.0),
            display_opacity: opacity,
        }
    }
}

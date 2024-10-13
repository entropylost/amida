use radiance::TuningSettings;

use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub world_size: [u32; 2],
    pub pixel_size: u32,
    pub dpi: f64,
    pub bounce_cascades: CascadeSettings,
    pub bounce_tuning: TuningSettings,
    pub cascades: CascadeSettings,
    pub display_tuning: TuningSettings,
    pub merge_variant: usize,
    pub num_bounces: usize,
    pub paused: bool,
    pub run_final: bool,
    pub show_diff: bool,
    pub raw_radiance: bool,
    pub display_level: u32,
    pub brush_radius: f32,
    pub draw_square: bool,
    pub materials: String,
    pub brushes: HashMap<BrushInput, Brush>,
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
            bounce_tuning: TuningSettings {
                block_sizes: [
                    (vec![0], [4, 8, 4]),
                    (vec![1, 2], [16, 4, 2]),
                    (vec![3, 4, 5, 6], [32, 2, 2]),
                ]
                .into_iter()
                .collect(),
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
            display_tuning: TuningSettings {
                block_sizes: [
                    (vec![0, 1], [4, 8, 4]),
                    (vec![2, 3], [16, 4, 2]),
                    (vec![4, 5, 6, 7], [32, 2, 2]),
                ]
                .into_iter()
                .collect(),
            },
            merge_variant: 0,
            num_bounces: 0,
            paused: false,
            run_final: true,
            show_diff: false,
            raw_radiance: false,
            display_level: 0,
            brush_radius: 5.0,
            draw_square: false,
            materials: "materials.ron".to_string(),
            brushes: [
                (
                    BrushInput::Mouse(MouseButton::Left),
                    Brush::Random(vec![
                        "wall1".to_string(),
                        "wall2".to_string(),
                        "wall3".to_string(),
                    ]),
                ),
                (
                    BrushInput::Mouse(MouseButton::Right),
                    Brush::Single("empty".to_string()),
                ),
                (
                    BrushInput::Mouse(MouseButton::Middle),
                    Brush::Single("light".to_string()),
                ),
                (
                    BrushInput::Mouse(MouseButton::Back),
                    Brush::Single("redglass".to_string()),
                ),
                (
                    BrushInput::Mouse(MouseButton::Forward),
                    Brush::Single("blueglass".to_string()),
                ),
            ]
            .into_iter()
            .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrushInput {
    Mouse(MouseButton),
    Key(KeyCode),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Brush {
    Single(String),
    Random(Vec<String>),
}
impl Brush {
    pub fn as_slice(&self) -> &[String] {
        match self {
            Self::Single(value) => std::slice::from_ref(value),
            Self::Random(values) => values,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Value)]
#[repr(C)]
pub struct LoadedMaterial {
    pub emissive: Vec3<f32>,
    pub diffuse: Vec3<f32>,
    pub opacity: Vec3<f32>,
    pub display_emissive: Vec3<f32>,
    pub display_diffuse: Vec3<f32>,
    pub display_opacity: Vec3<f32>,
}
impl From<Material> for LoadedMaterial {
    fn from(material: Material) -> Self {
        // Cases:
        // Has diffuse: Then probably is normal material. Opacity is display opacity is 1.0. Display_diffuse is diffuse.
        // Has emissive: Probably is some sorta glowing thing. Opacity is 1.0. Display_diffuse is 1.0.
        // Has opacity: Probably is some sorta transparent thing. Everything else is 0.
        // Has display_emissive: Probably is background. Everything else is 0.
        // Has display_diffuse: Probably is background. Everything else is 0.

        let zero = Vec3::splat(0.0);
        let one = Vec3::splat(1.0);

        let emissive = material.emissive.as_vec3();
        let diffuse = material.diffuse.as_vec3();
        let opacity = material.opacity.as_vec3();
        let display_emissive = material.display_emissive.as_vec3();
        let display_diffuse = material.display_diffuse.as_vec3();
        let display_opacity = material.display_opacity.as_vec3();

        if let Some(diffuse) = diffuse {
            let opacity = opacity.unwrap_or(one);
            Self {
                emissive: emissive.unwrap_or(zero),
                diffuse,
                opacity,
                display_emissive: display_emissive.unwrap_or(zero),
                display_diffuse: display_diffuse.unwrap_or(diffuse),
                display_opacity: display_opacity.unwrap_or(opacity),
            }
        } else if let Some(emissive) = emissive {
            let opacity = opacity.unwrap_or(one);
            Self {
                emissive,
                diffuse: zero,
                opacity,
                display_emissive: display_emissive.unwrap_or(zero),
                display_diffuse: display_diffuse.unwrap_or(one),
                display_opacity: display_opacity.unwrap_or(opacity),
            }
        } else if let Some(opacity) = opacity {
            Self {
                emissive: zero,
                diffuse: zero,
                opacity,
                display_emissive: display_emissive.unwrap_or(zero),
                display_diffuse: display_diffuse.unwrap_or(one),
                display_opacity: display_opacity.unwrap_or(opacity),
            }
        } else if let Some(display_emissive) = display_emissive {
            Self {
                emissive: zero,
                diffuse: zero,
                opacity: zero,
                display_emissive,
                display_diffuse: display_diffuse.unwrap_or(zero),
                display_opacity: display_opacity.unwrap_or(zero),
            }
        } else if let Some(display_diffuse) = display_diffuse {
            Self {
                emissive: zero,
                diffuse: zero,
                opacity: zero,
                display_emissive: zero,
                display_diffuse,
                display_opacity: display_opacity.unwrap_or(zero),
            }
        } else {
            Self {
                emissive: zero,
                diffuse: zero,
                opacity: zero,
                display_emissive: zero,
                display_diffuse: one,
                display_opacity: display_opacity.unwrap_or(zero),
            }
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MaterialVector {
    Repeat(f32),
    Vector(FVec3),
    #[default]
    None,
}
impl MaterialVector {
    pub fn as_vec3(&self) -> Option<Vec3<f32>> {
        match self {
            Self::Repeat(value) => Some(Vec3::splat(*value)),
            Self::Vector(value) => Some((*value).into()),
            Self::None => None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Material {
    pub emissive: MaterialVector,
    pub diffuse: MaterialVector,
    pub opacity: MaterialVector,
    pub display_emissive: MaterialVector,
    pub display_diffuse: MaterialVector,
    pub display_opacity: MaterialVector,
}

pub type Materials = HashMap<String, Material>;

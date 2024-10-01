use trace::Interval;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CascadeSettings {
    pub base_interval: (f32, f32),
    pub base_probe_spacing: f32,
    pub base_size: CascadeSize,
    pub num_cascades: u32,
    pub spatial_factor: u32, // default 1
    pub angular_factor: u32, // default 2
}
impl CascadeSettings {
    pub fn spacing(&self) -> u32 {
        1 << self.spatial_factor
    }
    pub fn branches(&self) -> u32 {
        1 << self.angular_factor
    }
    #[tracked]
    pub fn facing_direction(&self, facing: Expr<u32>, level: Expr<u32>) -> Expr<Vec2<f32>> {
        let angle = (facing.cast_f32() + 0.5_f32)
            / (self.base_size.facings << (self.angular_factor * level)).cast_f32()
            * TAU;
        Vec2::expr(angle.cos(), angle.sin())
    }
    #[tracked]
    pub fn probe_location(&self, probe: Expr<Vec2<u32>>, level: Expr<u32>) -> Expr<Vec2<f32>> {
        (probe.cast_f32() + 0.5) * self.probe_spacing(level)
    }
    #[tracked]
    pub fn probe_spacing(&self, level: Expr<u32>) -> Expr<f32> {
        self.base_probe_spacing * (1 << (self.spatial_factor * level)).cast_f32()
    }
    #[tracked]
    pub fn interval_end(&self, level: Expr<u32>) -> Expr<f32> {
        self.base_interval.1 * (1_u32 << (self.angular_factor * level)).cast_f32()
    }
    #[tracked]
    pub fn interval(&self, level: Expr<u32>) -> Expr<Interval> {
        Vec2::expr(
            if level == 0 {
                self.base_interval.0.expr()
            } else {
                self.interval_end(level - 1)
            },
            self.interval_end(level),
        )
    }
    #[tracked]
    pub fn probe_count(&self, level: Expr<u32>) -> Expr<Vec2<u32>> {
        self.base_size.probes >> (level * self.spatial_factor)
    }
    #[tracked]
    pub fn facing_count(&self, level: Expr<u32>) -> Expr<u32> {
        self.base_size.facings << (level * self.angular_factor)
    }
    pub fn level_size(&self, level: u32) -> CascadeSize {
        CascadeSize {
            probes: Vec2::new(
                self.base_size.probes.x >> (level * self.spatial_factor), // .max(1)
                self.base_size.probes.y >> (level * self.spatial_factor), // TODO: That'd break cascade_total_size.
            ),
            facings: self.base_size.facings << (level * self.angular_factor),
        }
    }
    #[tracked]
    pub fn level_size_expr(&self, level: Expr<u32>) -> Expr<CascadeSize> {
        CascadeSize::from_comps_expr(CascadeSizeComps {
            probes: self.probe_count(level),
            facings: self.facing_count(level),
        })
    }
    pub fn cascade_total_size(&self) -> u32 {
        assert!(self.spatial_factor * 2 >= self.angular_factor); // So they cancel out. Still not quite exact.
        self.base_size.probes.x * self.base_size.probes.y * self.base_size.facings
    }
    #[tracked]
    pub fn bilinear_samples(
        &self,
        probe: Expr<Vec2<u32>>,
        next_level: Expr<u32>,
    ) -> Expr<BilinearSamples> {
        let next_level_probe_location = ((probe.cast_f32() + 0.5) / self.spacing() as f32) - 0.5;
        let next_level_probe_location = next_level_probe_location.clamp(
            0.0,
            (self.base_size.probes >> (next_level * self.spatial_factor)).cast_f32() - 1.0,
        );
        let base_index = next_level_probe_location.floor().cast_u32();
        let fract = next_level_probe_location.fract();
        BilinearSamples::from_comps_expr(BilinearSamplesComps { base_index, fract })
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Value, Serialize, Deserialize)]
pub struct CascadeSize {
    #[serde(with = "to_glam")]
    pub probes: Vec2<u32>,
    pub facings: u32,
}

mod to_glam {
    use crate::Vec2;
    use glam::UVec2;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(a: &Vec2<u32>, s: S) -> Result<S::Ok, S::Error> {
        UVec2::new(a.x, a.y).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec2<u32>, D::Error> {
        let UVec2 { x, y } = UVec2::deserialize(d)?;
        Ok(Vec2::new(x, y))
    }
}

pub struct CascadeStorage<T: Value> {
    settings: CascadeSettings,
    buffer: Buffer<T>,
}
impl<T: Value> CascadeStorage<T> {
    pub fn new(settings: CascadeSettings) -> Self {
        let buffer =
            DEVICE.create_buffer((settings.cascade_total_size() * settings.num_cascades) as usize);
        Self { settings, buffer }
    }
    #[tracked]
    pub fn to_index(&self, ray: Expr<RayLocation>) -> Expr<u32> {
        let cascade_total_size = self.settings.cascade_total_size();
        let linear_index = ray.probe.x + ray.probe.y * self.settings.probe_count(ray.level).x;
        // Other way seems to run slightly slower.
        ray.level * cascade_total_size
            + linear_index * self.settings.facing_count(ray.level)
            + ray.facing // * self.settings.probe_count(ray.level).reduce_prod()
    }
    #[tracked]
    pub fn read(&self, ray: Expr<RayLocation>) -> Expr<T> {
        // let a = ray.cascade < self.settings.num_cascades;
        // lc_assert!(a);
        // let b = ray.probe.x
        //     < self.settings.base_size.probes.x >> (ray.cascade * self.settings.spatial_factor);
        // lc_assert!(b);
        // let c = ray.probe.y
        //     < self.settings.base_size.probes.y >> (ray.cascade * self.settings.spatial_factor);
        // lc_assert!(c);
        // let d = ray.direction
        //     < self.settings.base_size.directions << (ray.cascade * self.settings.angular_factor);
        // lc_assert!(d);

        self.buffer.read(self.to_index(ray))
    }
    pub fn write(&self, ray: Expr<RayLocation>, value: Expr<T>) {
        self.buffer.write(self.to_index(ray), value);
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Value)]
pub struct RayLocation {
    pub probe: Vec2<u32>,
    pub facing: u32,
    pub level: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Value)]
pub struct BilinearSamples {
    pub base_index: Vec2<u32>,
    pub fract: Vec2<f32>,
}
impl BilinearSamplesExpr {
    #[tracked]
    pub fn sample(&self, index: Expr<u32>) -> (Expr<Vec2<u32>>, Expr<f32>) {
        let fract = self.fract;
        let weights = <[_; 4]>::from_elems_expr([
            (1.0 - fract.x) * (1.0 - fract.y),
            fract.x * (1.0 - fract.y),
            (1.0 - fract.x) * fract.y,
            fract.x * fract.y,
        ]);
        let indices = [
            Vec2::new(0, 0),
            Vec2::new(1, 0),
            Vec2::new(0, 1),
            Vec2::new(1, 1),
        ]
        .expr();
        (indices[index] + self.base_index, weights[index])
    }
}

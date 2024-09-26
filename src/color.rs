use super::*;

pub type Radiance = Vec3<f32>;
pub type OpticalDepth = Vec3<f32>;
pub type Transmittance = Vec3<f32>;

pub fn opacity_to_optical_depth(opacity: Vec3<f32>) -> OpticalDepth {
    (-opacity).map(f32::exp)
}

#[tracked]
pub fn opacity_to_optical_depth_expr(opacity: Expr<Vec3<f32>>) -> Expr<OpticalDepth> {
    (-opacity).exp()
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Value)]
pub struct Color {
    pub radiance: Radiance,
    pub optical_depth: OpticalDepth,
}
impl ColorExpr {
    #[tracked]
    pub fn as_fluence(&self, segment_size: Expr<f32>) -> Expr<Fluence> {
        let color = self.self_;
        let transmittance = color.optical_depth.powf(luisa::max(segment_size, 0.0001));
        Fluence::from_comps_expr(FluenceComps {
            radiance: color.radiance * (1.0 - transmittance),
            transmittance,
        })
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Value)]
pub struct Fluence {
    pub radiance: Radiance,
    pub transmittance: Transmittance,
}

impl Fluence {
    pub fn transparent() -> Self {
        Self {
            radiance: Radiance::splat(0.0),
            transmittance: Transmittance::splat(1.0),
        }
    }
}

impl FluenceExpr {
    #[tracked]
    pub fn over(self, far: Expr<Fluence>) -> Expr<Fluence> {
        let near = self.self_;
        Fluence::from_comps_expr(FluenceComps {
            radiance: near.radiance + near.transmittance * far.radiance,
            transmittance: near.transmittance * far.transmittance,
        })
    }
    #[tracked]
    pub fn over_color(self, far: Expr<Radiance>) -> Expr<Radiance> {
        let near = self.self_;
        near.radiance + near.transmittance * far
    }
}

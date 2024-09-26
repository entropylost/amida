use super::*;

pub type Radiance = Vec3<f32>;
pub type OpticalDepth = Vec3<f32>;
pub type Transmittance = Vec3<f32>;
pub type Opacity = Vec3<f32>;

#[expect(unused)]
pub fn opacity_to_optical_depth(opacity: Opacity) -> OpticalDepth {
    (-opacity).map(f32::exp)
}

#[tracked]
#[expect(unused)]
pub fn opacity_to_optical_depth_expr(opacity: Expr<Opacity>) -> Expr<OpticalDepth> {
    (-opacity).exp()
}

#[expect(unused)]
pub fn optical_depth_to_opacity(optical_depth: OpticalDepth) -> Opacity {
    -optical_depth.map(f32::ln)
}

#[tracked]
#[expect(unused)]
pub fn optical_depth_to_opacity_expr(optical_depth: Expr<OpticalDepth>) -> Expr<Opacity> {
    -optical_depth.ln()
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Value)]
pub struct Color {
    pub radiance: Radiance,
    pub opacity: Opacity,
}
impl ColorExpr {
    #[tracked]
    pub fn as_fluence(&self, segment_size: Expr<f32>) -> Expr<Fluence> {
        let color = self.self_;
        let transmittance = (-color.opacity * segment_size).exp();
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

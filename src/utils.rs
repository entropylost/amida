use super::*;

// https://github.com/markjarzynski/PCG3D/blob/master/pcg3d.hlsl
#[tracked]
pub fn pcg3d(v: Expr<Vec3<u32>>) -> Expr<Vec3<u32>> {
    let v = v.var();
    *v = v * 1664525u32 + 1013904223u32;

    *v.x += v.y * v.z;
    *v.y += v.z * v.x;
    *v.z += v.x * v.y;

    *v ^= v >> 16u32;

    *v.x += v.y * v.z;
    *v.y += v.z * v.x;
    *v.z += v.x * v.y;

    **v
}

#[tracked]
pub fn pcg3df(v: Expr<Vec3<u32>>) -> Expr<Vec3<f32>> {
    pcg3d(v).cast_f32() / u32::MAX as f32
}

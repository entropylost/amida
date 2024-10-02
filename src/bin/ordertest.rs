use luisa::lang::functions::{block_id, sync_block, thread_id};
use luisa::lang::types::shared::Shared;
use luisa::lang::types::vector::{Vec2, Vec3};
use sefirot::prelude::*;
use sefirot::utils::Singleton;
use sefirot_testbed::App;

#[tracked]
fn color(time: Expr<u32>) -> Expr<Vec3<f32>> {
    Vec3::splat_expr((time.cast_f32() / (512 * 512 / 8 / 8) as f32).powf(1.0_f32 / 2.2))
}

fn main() {
    let app = App::new("Order test", [512, 512])
        .scale(4)
        .dpi_override(2.0)
        .init();

    let block_dim = 512_u32 / 8;

    let access_index = Singleton::<u32>::new();

    let kernel = DEVICE.create_kernel::<fn()>(&track!(|| {
        set_block_size([8, 8, 1]);

        let di = thread_id().xy()
            + Vec2::expr(
                block_id().x % 4 + (block_id().x / 4_u32 / block_dim) * 4,
                (block_id().x / 4) % block_dim,
            ) * 8;

        let time_shared = Shared::<u32>::new(1);

        if (thread_id() == Vec3::splat(0)).all() {
            let time = access_index.atomic().fetch_add(1);
            time_shared.write(0, time);
        }
        sync_block();

        let time = time_shared.read(0);
        let color = color(time);
        app.display().write(di, color);
    }));

    app.run(|_rt, scope| {
        (
            access_index.write_host(0),
            kernel.dispatch_async([512 * block_dim, 8, 1]),
        )
            .execute_in(&scope);
    })
}

use image::ImageReader;
use utils::pcg_host;

use super::*;

pub struct World {
    pub size: [u32; 2],
    pub emissive: Tex2d<Radiance>,
    pub diffuse: Tex2d<Diffuse>,
    pub opacity: Tex2d<Opacity>,
    pub display_emissive: Tex2d<Radiance>,
    pub display_diffuse: Tex2d<Diffuse>,
    pub display_opacity: Tex2d<Opacity>,
}

const PAGENAME: Tag = Tag::Unknown(285);

impl World {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            size: [width, height],
            emissive: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
            diffuse: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
            opacity: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
            display_emissive: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
            display_diffuse: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
            display_opacity: DEVICE.create_tex2d(PixelStorage::Float4, width, height, 1),
        }
    }
    pub fn width(&self) -> u32 {
        self.size[0]
    }
    pub fn height(&self) -> u32 {
        self.size[1]
    }
    pub fn load_default(&self) {
        DEVICE
            .create_kernel::<fn()>(&track!(|| {
                self.display_diffuse
                    .write(dispatch_id().xy(), Vec3::splat_expr(1.0));
            }))
            .dispatch([self.width(), self.height(), 1]);
    }
    pub fn load(&self, path: impl AsRef<Path> + Copy) {
        let file = File::open(path.as_ref().with_extension("tiff")).unwrap();
        let mut file = TiffDecoder::new(file).unwrap();

        let base_colortype = file.colortype().unwrap();
        let is_rgba = match base_colortype {
            ColorType::RGB(32) => false,
            ColorType::RGBA(32) => true,
            _ => panic!("Unsupported color type"),
        };
        let stride = if is_rgba { 4 } else { 3 };

        let staging_buffer =
            DEVICE.create_buffer::<f32>(stride * (self.width() * self.height()) as usize);
        let staging_kernel = DEVICE.create_kernel::<fn(Tex2d<Radiance>)>(&track!(|texture| {
            let index = stride as u32 * (dispatch_id().x + dispatch_id().y * self.width());
            let value = Vec3::expr(
                staging_buffer.read(index),
                staging_buffer.read(index + 1),
                staging_buffer.read(index + 2),
            );
            texture.write(dispatch_id().xy(), value);
        }));

        let mut load = |name: &str, texture: &Tex2d<Radiance>| {
            assert_eq!(
                file.get_tag_ascii_string(PAGENAME)
                    .as_deref()
                    .unwrap_or(name),
                name,
                "Layer names do not match"
            );
            assert_eq!(file.colortype().unwrap(), base_colortype);
            assert_eq!(file.dimensions().unwrap(), (self.width(), self.height()));
            let image = file.read_image().unwrap();
            let DecodingResult::F32(image) = image else {
                unreachable!()
            };
            (
                staging_buffer.copy_from_async(&image),
                staging_kernel.dispatch_async([self.width(), self.height(), 1], texture),
            )
                .chain()
                .execute();
            if file.more_images() {
                file.next_image().unwrap();
            }
        };
        // Generally viewed in reverse.
        load("display_opacity", &self.display_opacity);
        load("display_diffuse", &self.display_diffuse);
        load("display_emissive", &self.display_emissive);
        load("opacity", &self.opacity);
        load("diffuse", &self.diffuse);
        load("emissive", &self.emissive);
    }
    pub fn save(&self, path: impl AsRef<Path> + Copy) {
        let staging_buffer =
            DEVICE.create_buffer::<f32>(3 * (self.width() * self.height()) as usize);
        let staging_kernel = DEVICE.create_kernel::<fn(Tex2d<Radiance>)>(&track!(|texture| {
            let index = 3 * (dispatch_id().x + dispatch_id().y * self.width());
            let value = texture.read(dispatch_id().xy());
            staging_buffer.write(index, value.x);
            staging_buffer.write(index + 1, value.y);
            staging_buffer.write(index + 2, value.z);
        }));

        let mut staging_host = vec![0.0_f32; 3 * (self.width() * self.height()) as usize];

        let file = File::create(path.as_ref()).unwrap();
        let mut file = TiffEncoder::new(file).unwrap();

        let mut save = |name: &str, texture: &Tex2d<Radiance>| {
            (
                staging_kernel.dispatch_async([self.width(), self.height(), 1], texture),
                staging_buffer.copy_to_async(&mut staging_host),
            )
                .chain()
                .execute();
            let mut image = file
                .new_image::<colortype::RGB32Float>(self.width(), self.height())
                .unwrap();
            image
                .encoder() // PageName
                .write_tag(PAGENAME, name)
                .unwrap();
            image.write_data(&staging_host).unwrap();
        };
        // Generally viewed in reverse.
        save("display_opacity", &self.display_opacity);
        save("display_diffuse", &self.display_diffuse);
        save("display_emissive", &self.display_emissive);
        save("opacity", &self.opacity);
        save("diffuse", &self.diffuse);
        save("emissive", &self.emissive);
    }

    pub fn write_pixel(&self, pos: Expr<Vec2<u32>>, material: Expr<LoadedMaterial>) {
        self.emissive.write(pos, material.emissive);
        self.diffuse.write(pos, material.diffuse);
        self.opacity.write(pos, material.opacity);
        self.display_emissive.write(pos, material.display_emissive);
        self.display_diffuse.write(pos, material.display_diffuse);
        self.display_opacity.write(pos, material.display_opacity);
    }

    pub fn load_palette(
        &self,
        path: impl AsRef<Path> + Copy,
        palette: Palette,
        materials: &[(String, LoadedMaterial)],
    ) {
        let palette = palette
            .into_iter()
            .map(|(color, brush)| {
                let color = csscolorparser::parse(&color).unwrap().to_rgba8();
                let brush = brush
                    .as_slice()
                    .iter()
                    .map(|x| {
                        materials
                            .iter()
                            .enumerate()
                            .find(|(_, (name, _))| name == x)
                            .map(|(i, _)| i as u32)
                            .unwrap()
                    })
                    .collect::<Vec<_>>();
                (color, brush)
            })
            .collect::<HashMap<_, _>>();

        let image = ImageReader::open(path)
            .unwrap()
            .decode()
            .unwrap()
            .into_rgba8();

        assert!(image.width() == self.width() && image.height() == self.height());

        let staging_buffer =
            DEVICE.create_buffer::<f32>(3 * (self.width() * self.height()) as usize);
        let staging_kernel = DEVICE.create_kernel::<fn(Tex2d<Radiance>)>(&track!(|texture| {
            let index = 3 * (dispatch_id().x + dispatch_id().y * self.width());
            let value = Vec3::expr(
                staging_buffer.read(index),
                staging_buffer.read(index + 1),
                staging_buffer.read(index + 2),
            );
            texture.write(dispatch_id().xy(), value);
        }));

        let load = |texture: &Tex2d<Radiance>, f: fn(&LoadedMaterial) -> Vec3<f32>| {
            let data = image
                .enumerate_pixels()
                .flat_map(|(i, j, x)| {
                    let brush = palette.get(&x.0).unwrap_or_else(|| {
                        panic!("Color not found: {:?}", x.0);
                    });
                    let material = brush[pcg_host((i << 16) + j) as usize % brush.len()];
                    <[f32; 3]>::from(f(&materials[material as usize].1))
                })
                .collect::<Vec<_>>();
            staging_buffer.copy_from(&data);
            staging_kernel.dispatch([self.width(), self.height(), 1], texture);
        };
        load(&self.emissive, |x| x.emissive);
        load(&self.diffuse, |x| x.diffuse);
        load(&self.opacity, |x| x.opacity);
        load(&self.display_emissive, |x| x.display_emissive);
        load(&self.display_diffuse, |x| x.display_diffuse);
        load(&self.display_opacity, |x| x.display_opacity);
    }
}

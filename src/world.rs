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
    pub fn load(&self, path: impl AsRef<Path> + Copy) {
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

        let file = File::open(path.as_ref().with_extension("tiff")).unwrap();
        let mut file = TiffDecoder::new(file).unwrap();

        let mut load = |name: &str, texture: &Tex2d<Radiance>| {
            assert_eq!(&file.get_tag_ascii_string(PAGENAME).unwrap(), name);
            assert_eq!(file.colortype().unwrap(), ColorType::RGB(32));
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
}

use crate::RgbaBuffer;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::Path;

#[repr(C)]
#[derive(Debug, Default)]
struct FbVarScreenInfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: FbBitField,
    green: FbBitField,
    blue: FbBitField,
    transp: FbBitField,
    // ... remaining fields padded
    _padding: [u32; 14],
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct FbBitField {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Debug)]
struct FbFixScreenInfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    type_: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    // ... remaining fields
    _padding: [u8; 32],
}

pub struct FramebufferDevice {
    _fd: File,
    mmap_ptr: *mut u8,
    mmap_len: usize,
    width: u32,
    height: u32,
    stride: u32,
    bpp: u32,
    red_offset: u32,
    green_offset: u32,
    blue_offset: u32,
    alpha_offset: u32,
}

unsafe impl Send for FramebufferDevice {}
unsafe impl Sync for FramebufferDevice {}

impl FramebufferDevice {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let file = OpenOptions::new().read(true).open(path)?;
        let fd = file.as_raw_fd();

        let mut var_info: FbVarScreenInfo = unsafe { std::mem::zeroed() };
        let ret = unsafe { crate::raw_ioctl(fd, 0x4600, &mut var_info as *mut _ as *mut libc::c_void) }; // FBIOGET_VSCREENINFO
        if ret < 0 {
            anyhow::bail!("FBIOGET_VSCREENINFO failed");
        }

        let mut fix_info: FbFixScreenInfo = unsafe { std::mem::zeroed() };
        let ret = unsafe { crate::raw_ioctl(fd, 0x4602, &mut fix_info as *mut _ as *mut libc::c_void) }; // FBIOGET_FSCREENINFO
        if ret < 0 {
            anyhow::bail!("FBIOGET_FSCREENINFO failed");
        }

        let mmap_len = fix_info.smem_len as usize;
        let mmap_ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                mmap_len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if mmap_ptr == libc::MAP_FAILED {
            anyhow::bail!("mmap failed for framebuffer");
        }

        Ok(Self {
            _fd: file,
            mmap_ptr: mmap_ptr as *mut u8,
            mmap_len,
            width: var_info.xres,
            height: var_info.yres,
            stride: fix_info.line_length,
            bpp: var_info.bits_per_pixel,
            red_offset: var_info.red.offset,
            green_offset: var_info.green.offset,
            blue_offset: var_info.blue.offset,
            alpha_offset: var_info.transp.offset,
        })
    }

    pub fn open_default() -> anyhow::Result<Self> {
        Self::open(Path::new("/dev/graphics/fb0"))
            .or_else(|_| Self::open(Path::new("/dev/fb0")))
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn capture(&self) -> anyhow::Result<RgbaBuffer> {
        let bytes_per_pixel = (self.bpp / 8) as usize;
        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);

        let pixels = unsafe {
            std::slice::from_raw_parts(self.mmap_ptr, self.mmap_len)
        };

        for y in 0..self.height as usize {
            let row_start = y * self.stride as usize;
            for x in 0..self.width as usize {
                let offset = row_start + x * bytes_per_pixel;
                if offset + bytes_per_pixel > pixels.len() { break; }

                let pixel = &pixels[offset..offset + bytes_per_pixel];
                let (r, g, b, a) = self.decode_pixel(pixel);
                rgba.extend_from_slice(&[r, g, b, a]);
            }
        }

        Ok(RgbaBuffer {
            data: rgba,
            width: self.width,
            height: self.height,
        })
    }

    fn decode_pixel(&self, pixel: &[u8]) -> (u8, u8, u8, u8) {
        let val = match self.bpp {
            32 => {
                (pixel[0] as u32)
                    | (pixel[1] as u32) << 8
                    | (pixel[2] as u32) << 16
                    | (pixel[3] as u32) << 24
            }
            16 => (pixel[0] as u32) | (pixel[1] as u32) << 8,
            _ => 0,
        };

        let r = ((val >> self.red_offset) & 0xFF) as u8;
        let g = ((val >> self.green_offset) & 0xFF) as u8;
        let b = ((val >> self.blue_offset) & 0xFF) as u8;
        let a = if self.bpp == 32 {
            ((val >> self.alpha_offset) & 0xFF) as u8
        } else {
            255
        };

        (r, g, b, a)
    }
}

impl Drop for FramebufferDevice {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.mmap_ptr as *mut libc::c_void, self.mmap_len);
        }
    }
}

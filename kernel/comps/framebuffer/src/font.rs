// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, vec::Vec};

use font8x8::UnicodeFonts;

/// A bitmap font.
///
/// Currently it's mainly used to draw texts on the framebuffer console. See
/// [`FramebufferConsole::set_font`].
///
/// [`FramebufferConsole::set_font`]: crate::FramebufferConsole::set_font
#[derive(Debug)]
pub struct BitmapFont {
    width: usize,
    height: usize,
    char_size: usize,
    bitmap: Box<[u8]>,
}

impl BitmapFont {
    /// Creates a new bitmap font.
    ///
    /// In a bitmap, a one in a given position indicates that the foreground pixel should be drawn
    /// at that row and column.
    ///
    /// The font should contain a set of characters, starting from the NUL character. Each
    /// character contains `height` rows and each row contains `width` columns. So a row contains
    /// `width` bits and occupies `width.div_ceil(u8::BITS as _)` bytes. Note that the least
    /// significant bit of a byte represents the smallest column number.
    ///
    /// # Panics
    ///
    /// This method will panic if the bitmap contains no characters or the last character in the
    /// bitmap is incomplete.
    pub fn new(width: usize, height: usize, bitmap: Box<[u8]>) -> Self {
        let row_size = width.div_ceil(u8::BITS as usize);
        let char_size = row_size.checked_mul(height).unwrap();

        assert_ne!(bitmap.len(), 0);
        assert_eq!(bitmap.len() % char_size, 0);

        Self {
            width,
            height,
            char_size,
            bitmap,
        }
    }

    /// Creates a new bitmap font that represents the basic 8x8 font.
    ///
    /// This is the default font for the framebuffer console.
    pub fn new_basic8x8() -> Self {
        const CHAR_COUNT: u32 = 0x7F;

        const FONT_WIDTH: usize = 8;
        const FONT_HEIGHT: usize = 8;

        let bitmap = (0..CHAR_COUNT)
            .flat_map(|ch| {
                font8x8::BASIC_FONTS
                    .get(char::from_u32(ch).unwrap())
                    .unwrap()
                    .into_iter()
            })
            .collect();

        Self::new(FONT_WIDTH, FONT_HEIGHT, bitmap)
    }

    /// Creates a new bitmap font with the specified `vpitch` value.
    ///
    /// This method is similar to [`BitmapFont::new`], except that each character in the bitmap
    /// contains `vpitch` rows. Note that the character height is still `height`, so the remaining
    /// rows in the bitmap will be ignored.
    ///
    /// # Panics
    ///
    /// This method will panic if `height` is smaller than `vpitch`.
    ///
    /// Besides, this method will panic for the same reasons specified in [`BitmapFont::new`].
    pub fn new_with_vpitch(
        width: usize,
        height: usize,
        vpitch: usize,
        mut bitmap: Vec<u8>,
    ) -> Self {
        if height == vpitch {
            return Self::new(width, height, bitmap.into_boxed_slice());
        }
        assert!(height < vpitch);

        let row_size = width.div_ceil(u8::BITS as usize);
        let char_size_old = row_size.checked_mul(vpitch).unwrap();
        let char_size_new = row_size.checked_mul(height).unwrap();

        assert_ne!(bitmap.len(), 0);
        assert_eq!(bitmap.len() % char_size_old, 0);

        let mut old_pos = char_size_old;
        let mut new_pos = char_size_new;
        while old_pos < bitmap.len() {
            bitmap.copy_within(old_pos..old_pos + char_size_new, new_pos);
            old_pos += char_size_old;
            new_pos += char_size_new;
        }

        bitmap.truncate(new_pos);

        Self::new(width, height, bitmap.into_boxed_slice())
    }

    /// Returns the width of the font.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Returns the height of the font.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Returns the bitmap of the specified character in the font.
    ///
    /// This method will return [`None`] if the font does not contain a bitmap for the specified
    /// character.
    pub fn char(&self, ch: u8) -> Option<BitmapChar> {
        let pos = (ch as usize) * self.char_size;
        let data = self.bitmap.get(pos..pos + self.char_size)?;

        Some(BitmapChar {
            font: self,
            char_data: data,
        })
    }
}

/// A bitmap of a character in a [`BitmapFont`].
#[derive(Debug)]
pub struct BitmapChar<'a> {
    font: &'a BitmapFont,
    char_data: &'a [u8],
}

impl<'a> BitmapChar<'a> {
    /// Iterates over the rows of the character bitmap.
    pub fn rows(&self) -> impl Iterator<Item = BitmapCharRow<'a>> {
        let row_size = self.font.width.div_ceil(u8::BITS as usize);
        self.char_data
            .chunks_exact(row_size)
            .map(|chunk| BitmapCharRow {
                font: self.font,
                row_data: chunk,
            })
    }
}

/// A bitmap of a row in a [`BitmapChar`].
#[derive(Debug)]
pub struct BitmapCharRow<'a> {
    font: &'a BitmapFont,
    row_data: &'a [u8],
}

impl BitmapCharRow<'_> {
    /// Iterates over the bits (i.e., the columns) of the row bitmap.
    pub fn bits(&self) -> impl Iterator<Item = bool> + '_ {
        (0..self.font.width).map(|i| {
            let nbyte = i / (u8::BITS as usize);
            let nbit = i % (u8::BITS as usize);
            self.row_data[nbyte] & (1 << nbit) != 0
        })
    }
}

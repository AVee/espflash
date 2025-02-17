use bytemuck::from_bytes;
use crate::elf::{CodeSegment, FirmwareImage};
use crate::image_format::{ImageHeader, SegmentHeader};

#[derive(Debug)]
pub struct EspFirmwareImage<'a> {
    pub image_data: &'a [u8],
}

impl<'a> EspFirmwareImage<'a> {
    pub fn new<'b: 'a>(image_data: &'b [u8]) -> Self {
        // TODO: Validate
        Self { image_data }
    }
}

#[derive(Debug)]
pub struct SectionIter<'a> {
    data: &'a [u8],
    pos: usize,
    remaining: u8,
}
impl<'a> Iterator for SectionIter<'a> {
    type Item = CodeSegment<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining > 0 {
            self.remaining -= 1;
            let segment: SegmentHeader = *from_bytes(&self.data[self.pos..self.pos + size_of::<SegmentHeader>()]);
            
            let result = Some(CodeSegment::new(
                segment.addr, 
                &self.data[self.pos + size_of::<SegmentHeader>() ..self.pos + size_of::<SegmentHeader>() + segment.length as usize]));
            self.pos = self.pos + segment.length as usize + size_of::<SegmentHeader>();
            result
        }
        else { 
            None
        }
    }
}

impl<'a> FirmwareImage<'a> for EspFirmwareImage<'a> {
    fn entry(&self) -> u32 {
        let header: ImageHeader = *from_bytes(&self.image_data[..size_of::<ImageHeader>()]);
        header.entry
    }

    fn segments(&self) -> Box<dyn Iterator<Item=CodeSegment<'_>> + '_> {
        let mut calc_bootloader_size = 0;
        let bootloader_header_size = size_of::<ImageHeader>();
        calc_bootloader_size += bootloader_header_size;

        let header: ImageHeader = *from_bytes(&self.image_data[..size_of::<ImageHeader>()]);
        Box::new(SectionIter { data: self.image_data, pos: calc_bootloader_size, remaining: header.segment_count })
    }
}
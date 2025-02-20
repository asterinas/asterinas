use align_ext::AlignExt;

use super::{addr::AddrSegment, link::LinkSegment, AttrOps, CMessageType};
use crate::{net::socket::netlink::message::CNetlinkMessageHeader, prelude::*, util::MultiRead};

#[derive(Debug)]
pub struct NlMsg {
    pub segments: Vec<Box<dyn NlMsgSegment>>,
}

impl NlMsg {
    pub fn read_from_user(reader: &mut dyn MultiRead) -> Result<Self> {
        let mut segments = Vec::new();

        while reader.remain() > 0 {
            let header = reader.read_val::<CNetlinkMessageHeader>()?;
            match CMessageType::try_from(header.type_)? {
                CMessageType::GETLINK => segments
                    .push(Box::new(LinkSegment::read_from_user(header, reader)?)
                        as Box<dyn NlMsgSegment>),
                CMessageType::GETADDR => segments
                    .push(Box::new(AddrSegment::read_from_user(header, reader)?)
                        as Box<dyn NlMsgSegment>),
                _ => todo!(),
            }
        }

        Ok(Self { segments })
    }

    pub fn total_len(&self) -> usize {
        self.segments
            .iter()
            .map(|segment| segment.header().len as usize)
            .sum()
    }
}

pub trait NlMsgSegment: Send + Sync + Debug {
    fn header(&self) -> &CNetlinkMessageHeader;
    fn header_mut(&mut self) -> &mut CNetlinkMessageHeader;
    fn as_any(&self) -> &dyn Any;
    fn adjust_header_len(&mut self) {
        let attrs_len = self
            .attrs()
            .iter()
            .map(|attr| attr.total_len_with_padding())
            .sum();
        self.header_mut().len = size_of::<CNetlinkMessageHeader>() + self.body_len() + attrs_len;
    }
    fn attrs(&self) -> &Vec<Box<dyn AttrOps>>;
    fn body_len(&self) -> usize;
}

pub trait ReadNlMsgSegmentFromUser: Sized {
    type Body: ReadBodyFromUser;

    fn new(header: CNetlinkMessageHeader, body: Self::Body, attrs: Vec<Box<dyn AttrOps>>) -> Self;

    fn read_from_user(header: CNetlinkMessageHeader, reader: &mut dyn MultiRead) -> Result<Self>
    where
        Error: From<
            <<<Self as ReadNlMsgSegmentFromUser>::Body as ReadBodyFromUser>::CType as TryInto<
                <Self as ReadNlMsgSegmentFromUser>::Body,
            >>::Error,
        >,
    {
        let (body, body_len) = <Self::Body as ReadBodyFromUser>::read_from_user(&header, reader)?;

        let attrs = {
            println!("total len = {}", header.len);
            println!(
                "header len = {}, body_len = {}",
                size_of_val(&header),
                body_len
            );
            let attrs_len = header.len as usize - size_of_val(&header) - body_len;

            Self::read_attrs(attrs_len, reader)?
        };

        let segment = Self::new(header, body, attrs);
        Ok(segment)
    }

    fn read_attrs(attrs_len: usize, reader: &mut dyn MultiRead) -> Result<Vec<Box<dyn AttrOps>>>;
}

pub trait ReadBodyFromUser: Sized {
    // The actual message should be `Self::CType`,
    // however, old Linux uses lagacy type(usually `CRtGenMessage`) here.
    // We should deal with both cases.
    // Ref: https://elixir.bootlin.com/linux/v6.13/source/net/core/rtnetlink.c#L2393
    type LegacyType: Pod;
    type CType: Pod + From<Self::LegacyType> + TryInto<Self>;

    fn validate_c_type(_header: &CNetlinkMessageHeader, _c_type: &Self::CType) -> Result<()> {
        Ok(())
    }

    fn read_from_user(
        header: &CNetlinkMessageHeader,
        reader: &mut dyn MultiRead,
    ) -> Result<(Self, usize)>
    where
        Error: From<<<Self as ReadBodyFromUser>::CType as TryInto<Self>>::Error>,
    {
        let max_len = header.len as usize - size_of_val(header);

        let (c_type, read_size) = if max_len < size_of::<Self::CType>() {
            let lagacy = reader.read_val::<Self::LegacyType>()?;
            (Self::CType::from(lagacy), size_of::<Self::LegacyType>())
        } else {
            let c_type = reader.read_val::<Self::CType>()?;
            (c_type, size_of::<Self::CType>())
        };

        Self::validate_c_type(header, &c_type)?;

        let body = c_type.try_into()?;
        Ok((body, read_size))
    }
}

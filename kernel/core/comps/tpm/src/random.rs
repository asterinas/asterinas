// SPDX-License-Identifier: MPL-2.0

use alloc::{vec, vec::Vec};

use p256::{PublicKey, SecretKey, ecdh, elliptic_curve::sec1::ToEncodedPoint};
use tpm_core::{
    AuthSession, SA_ENCRYPT, TpmTag,
    auth::{CMD_SESSION_LEN, HEADER_LEN, ST_SESSIONS},
    chip::{ChipTransport, CtxIo},
    crypto::{NonceSource, SHA256_LEN, Sha256Ctx},
    cursor::Cursor,
    module::{ContextIo, IoErr},
    session,
    types::{TPM_BUFSIZE, TPM_MAX_RNG_DATA},
};
use zeroize::Zeroizing;

use crate::extcrypto::{ExtAesCfb, ExtHmacSha256, ExtSha256};

const CC_CREATE_PRIMARY: u32 = 0x0000_0131;
const CC_START_AUTH_SESSION: u32 = 0x0000_0176;
const CC_GET_RANDOM: u32 = 0x0000_017b;
const RH_NULL: u32 = 0x4000_0007;
const RS_PW: u32 = 0x4000_0009;
const SECRET_KEY_ATTEMPTS: usize = 16;
const MAX_RANDOM_COMMANDS: usize = 6;
const NULL_PRIMARY_TEMPLATE: [u8; 26] = [
    0x00, 0x23, 0x00, 0x0b, 0x00, 0x03, 0x04, 0x72, 0x00, 0x00, 0x00, 0x06, 0x00, 0x80, 0x00, 0x43,
    0x00, 0x10, 0x00, 0x03, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00,
];
const PUBLIC_PREFIX_SIZE: usize = NULL_PRIMARY_TEMPLATE.len() - 4;

pub(super) struct TpmRandom {
    primary: Option<Primary>,
}

impl TpmRandom {
    pub(super) fn new() -> Self {
        Self { primary: None }
    }

    pub(super) fn read<T: ChipTransport, R: NonceSource>(
        &mut self,
        io: &mut CtxIo<T>,
        out: &mut [u8],
        rng: &mut R,
    ) -> Result<usize, IoErr> {
        if out.is_empty() || out.len() > TPM_MAX_RNG_DATA {
            return Err(IoErr::BadCommand);
        }
        if self.primary.is_none() {
            self.primary = Some(Primary::create(io)?);
        }
        let primary = self.primary.as_ref().unwrap();
        let (mut session, guard) = primary.start_session(io, rng)?;
        read_session(&mut session, guard.io, out, rng)
    }
}

struct Primary {
    context: Zeroizing<Vec<u8>>,
    public_key: PublicKey,
}

struct LoadedHandle<'a, T: ChipTransport> {
    io: &'a mut CtxIo<T>,
    handle: u32,
}

impl<T: ChipTransport> Drop for LoadedHandle<'_, T> {
    fn drop(&mut self) {
        self.io.flush(self.handle);
    }
}

impl Primary {
    fn create<T: ChipTransport>(io: &mut CtxIo<T>) -> Result<Self, IoErr> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&RH_NULL.to_be_bytes());
        payload.extend_from_slice(&9u32.to_be_bytes());
        payload.extend_from_slice(&RS_PW.to_be_bytes());
        payload.extend_from_slice(&[0; 5]);
        payload.extend_from_slice(&[0, 4, 0, 0, 0, 0]);
        payload.extend_from_slice(&(NULL_PRIMARY_TEMPLATE.len() as u16).to_be_bytes());
        payload.extend_from_slice(&NULL_PRIMARY_TEMPLATE);
        payload.extend_from_slice(&[0; 6]);
        let command = command(ST_SESSIONS, CC_CREATE_PRIMARY, &payload);
        let mut response = vec![0u8; TPM_BUFSIZE];
        let n = execute(io, &command, &mut response)?;
        let mut cursor = Cursor::new();
        if !cursor.skip(&response[..n], HEADER_LEN) {
            return Err(IoErr::Protocol);
        }
        let handle = cursor.read_be32(&response[..n]).ok_or(IoErr::Protocol)?;
        if handle >> 24 != 0x80 {
            return Err(IoErr::Protocol);
        }
        let guard = LoadedHandle { io, handle };
        let param_len = cursor.read_be32(&response[..n]).ok_or(IoErr::Protocol)? as usize;
        let params = cursor
            .read_bytes(&response[..n], param_len)
            .ok_or(IoErr::Protocol)?;
        let public_key = parse_public(params)?;
        let mut context = Zeroizing::new(vec![0u8; TPM_BUFSIZE]);
        let n = guard.io.save(handle, &mut context, 0)?;
        context.truncate(n);
        Ok(Self {
            context,
            public_key,
        })
    }

    fn start_session<'a, T: ChipTransport, R: NonceSource>(
        &self,
        io: &'a mut CtxIo<T>,
        rng: &mut R,
    ) -> Result<(AuthSession, LoadedHandle<'a, T>), IoErr> {
        let secret = secret_key(rng)?;
        let public = secret.public_key().to_encoded_point(false);
        let peer = self.public_key.to_encoded_point(false);
        let shared = ecdh::diffie_hellman(secret.to_nonzero_scalar(), self.public_key.as_affine());
        let mut digest = ExtSha256::new();
        digest.update(&1u32.to_be_bytes());
        digest.update(shared.raw_secret_bytes());
        digest.update(b"SECRET\0");
        digest.update(public.x().ok_or(IoErr::Protocol)?);
        digest.update(peer.x().ok_or(IoErr::Protocol)?);
        let salt = Zeroizing::new(digest.finish());
        let nonce = rng.nonce();

        let (handle, consumed) = io.load(&self.context, 0)?;
        let primary = LoadedHandle { io, handle };
        if consumed != self.context.len() {
            return Err(IoErr::Protocol);
        }
        let mut payload = Vec::new();
        payload.extend_from_slice(&handle.to_be_bytes());
        payload.extend_from_slice(&RH_NULL.to_be_bytes());
        payload.extend_from_slice(&(nonce.len() as u16).to_be_bytes());
        payload.extend_from_slice(&nonce);
        payload.extend_from_slice(&68u16.to_be_bytes());
        for point in [public.x(), public.y()] {
            payload.extend_from_slice(&(SHA256_LEN as u16).to_be_bytes());
            payload.extend_from_slice(point.ok_or(IoErr::Protocol)?);
        }
        payload.extend_from_slice(&[0, 0, 6, 0, 128, 0, 0x43, 0, 0x0b]);
        let command = command(
            TpmTag::Tpm2NoSessions.code(),
            CC_START_AUTH_SESSION,
            &payload,
        );
        let mut response = [0u8; 64];
        let n = execute(primary.io, &command, &mut response)?;
        let mut cursor = Cursor::new();
        if !cursor.skip(&response[..n], HEADER_LEN) {
            return Err(IoErr::Protocol);
        }
        let session_handle = cursor.read_be32(&response[..n]).ok_or(IoErr::Protocol)?;
        drop(primary);
        if session_handle >> 24 != 0x02 {
            return Err(IoErr::Protocol);
        }
        let guard = LoadedHandle {
            io,
            handle: session_handle,
        };
        let peer_nonce = sized_bytes(&mut cursor, &response[..n])?;
        if cursor.pos() != n {
            return Err(IoErr::Protocol);
        }
        let peer_nonce = peer_nonce.try_into().map_err(|_| IoErr::Protocol)?;
        let session = AuthSession::open_session::<ExtHmacSha256>(
            session_handle,
            &salt[..],
            nonce,
            peer_nonce,
        );
        Ok((session, guard))
    }
}

fn read_session<T: ChipTransport, R: NonceSource>(
    session: &mut AuthSession,
    io: &mut CtxIo<T>,
    out: &mut [u8],
    rng: &mut R,
) -> Result<usize, IoErr> {
    let mut copied = 0;
    for _ in 0..MAX_RANDOM_COMMANDS {
        let mut response = Zeroizing::new(vec![0u8; TPM_BUFSIZE]);
        let requested = out.len() - copied;
        let command = random_command(session, rng, requested as u16);
        let n = execute(io, &command, &mut response)?;
        let bytes = random_response(session, &mut response[..n], requested)?;
        out[copied..copied + bytes.len()].copy_from_slice(bytes);
        copied += bytes.len();
        if copied == out.len() {
            break;
        }
    }
    if copied == 0 {
        Err(IoErr::Fatal)
    } else {
        Ok(copied)
    }
}

fn random_command<R: NonceSource>(
    session: &mut AuthSession,
    rng: &mut R,
    requested: u16,
) -> Vec<u8> {
    session.begin(rng, CC_GET_RANDOM, SA_ENCRYPT);
    let mut payload = Vec::new();
    payload.extend_from_slice(&(CMD_SESSION_LEN as u32).to_be_bytes());
    payload.extend_from_slice(&session.handle().to_be_bytes());
    payload.extend_from_slice(&(SHA256_LEN as u16).to_be_bytes());
    payload.extend_from_slice(&session.our_nonce());
    payload.push(session.attrs());
    payload.extend_from_slice(&(SHA256_LEN as u16).to_be_bytes());
    payload.extend_from_slice(&[0; SHA256_LEN]);
    payload.extend_from_slice(&requested.to_be_bytes());
    let mut command = command(ST_SESSIONS, CC_GET_RANDOM, &payload);
    let session_offset = HEADER_LEN + 4;
    let param_offset = session_offset + CMD_SESSION_LEN;
    session.finalize::<ExtSha256, ExtHmacSha256>(
        &mut command,
        session_offset,
        0,
        &[],
        param_offset,
        2,
    );
    command
}

fn random_response<'a>(
    session: &mut AuthSession,
    response: &'a mut [u8],
    requested: usize,
) -> Result<&'a [u8], IoErr> {
    let auth = session
        .check_response::<ExtSha256, ExtHmacSha256>(response, 0)
        .map_err(|_| IoErr::Integrity)?;
    let params = &mut response[auth.param_off..auth.param_off + auth.param_len];
    let mut cursor = Cursor::new();
    let len = cursor.read_be16(params).ok_or(IoErr::Protocol)? as usize;
    if len + 2 != params.len() {
        return Err(IoErr::Protocol);
    }
    let material = Zeroizing::new(session::cfb_material::<ExtHmacSha256>(
        session,
        &auth.tpm_nonce,
        &session.our_nonce(),
    ));
    session::decrypt_param(&ExtAesCfb, &material, &mut params[2..]);
    Ok(&params[2..2 + len.min(requested)])
}

fn parse_public(params: &[u8]) -> Result<PublicKey, IoErr> {
    let mut cursor = Cursor::new();
    let public = sized_bytes(&mut cursor, params)?;
    if public.get(..PUBLIC_PREFIX_SIZE) != Some(&NULL_PRIMARY_TEMPLATE[..PUBLIC_PREFIX_SIZE]) {
        return Err(IoErr::Protocol);
    }
    let mut cursor = Cursor::new();
    if !cursor.skip(public, PUBLIC_PREFIX_SIZE) {
        return Err(IoErr::Protocol);
    }
    let mut point = [0u8; 65];
    point[0] = 4;
    let (x, y) = point[1..].split_at_mut(SHA256_LEN);
    for coordinate in [x, y] {
        let value = sized_bytes(&mut cursor, public)?;
        if value.is_empty() || value.len() > SHA256_LEN {
            return Err(IoErr::Protocol);
        }
        coordinate[SHA256_LEN - value.len()..].copy_from_slice(value);
    }
    if cursor.pos() != public.len() {
        return Err(IoErr::Protocol);
    }
    PublicKey::from_sec1_bytes(&point).map_err(|_| IoErr::Protocol)
}

fn sized_bytes<'a>(cursor: &mut Cursor, bytes: &'a [u8]) -> Result<&'a [u8], IoErr> {
    let len = cursor.read_be16(bytes).ok_or(IoErr::Protocol)? as usize;
    cursor.read_bytes(bytes, len).ok_or(IoErr::Protocol)
}

fn secret_key<R: NonceSource>(rng: &mut R) -> Result<SecretKey, IoErr> {
    for _ in 0..SECRET_KEY_ATTEMPTS {
        let bytes = Zeroizing::new(rng.nonce());
        if let Ok(secret) = SecretKey::from_slice(&bytes[..]) {
            return Ok(secret);
        }
    }
    Err(IoErr::Fatal)
}

fn command(tag: u16, code: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
    bytes.extend_from_slice(&tag.to_be_bytes());
    bytes.extend_from_slice(&((HEADER_LEN + payload.len()) as u32).to_be_bytes());
    bytes.extend_from_slice(&code.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn execute<T: ChipTransport>(
    io: &mut CtxIo<T>,
    command: &[u8],
    response: &mut [u8],
) -> Result<usize, IoErr> {
    let n = io.exec_raw(command, response)?;
    let mut cursor = Cursor::new();
    let tag = cursor.read_be16(&response[..n]).ok_or(IoErr::Protocol)?;
    let len = cursor.read_be32(&response[..n]).ok_or(IoErr::Protocol)? as usize;
    let rc = cursor.read_be32(&response[..n]).ok_or(IoErr::Protocol)?;
    if (tag != TpmTag::Tpm2NoSessions.code() && tag != ST_SESSIONS) || len != n {
        return Err(IoErr::Protocol);
    }
    if rc != 0 {
        return Err(IoErr::Fatal);
    }
    Ok(n)
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::*;
    use crate::extcrypto::ExtRng;

    const RESPONSE: [u8; 89] = [
        0x80, 0x02, 0x00, 0x00, 0x00, 0x59, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00,
        0x04, 0x17, 0xe9, 0xa5, 0x5d, 0x00, 0x20, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44,
        0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44,
        0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x41, 0x00, 0x20, 0xbb, 0x52, 0x5b,
        0x0c, 0xbd, 0x57, 0x81, 0x03, 0xcb, 0x7e, 0xe3, 0x95, 0x37, 0xe0, 0x85, 0xc4, 0xdf, 0xf8,
        0x20, 0xca, 0xc7, 0xf7, 0x4a, 0xe5, 0xc3, 0x54, 0x55, 0x4a, 0xee, 0xf5, 0x92, 0xe6,
    ];

    fn pending_session(requested: u16) -> AuthSession {
        let mut session = AuthSession::open_session::<ExtHmacSha256>(
            0x0200_0000,
            &[0x55; 32],
            [0x11; 32],
            [0x22; 32],
        );
        let mut rng = ExtRng::new(|bytes: &mut [u8]| bytes.fill(0x33));
        random_command(&mut session, &mut rng, requested);
        session
    }

    #[ktest]
    fn decrypts_authenticated_random_response() {
        let mut session = pending_session(4);
        let mut response = RESPONSE;
        assert_eq!(
            random_response(&mut session, &mut response, 4),
            Ok(&[0xa1, 0xb2, 0xc3, 0xd4][..])
        );
    }

    #[ktest]
    fn rejects_modified_response_hmac() {
        let mut session = pending_session(4);
        let mut response = RESPONSE;
        response[88] ^= 1;
        let original = response;
        assert_eq!(
            random_response(&mut session, &mut response, 4),
            Err(IoErr::Integrity)
        );
        assert_eq!(response, original);
    }

    #[ktest]
    fn rejects_modified_ciphertext() {
        let mut session = pending_session(4);
        let mut response = RESPONSE;
        response[16] ^= 1;
        let original = response;
        assert_eq!(
            random_response(&mut session, &mut response, 4),
            Err(IoErr::Integrity)
        );
        assert_eq!(response, original);
    }

    #[ktest]
    fn rejects_truncated_response() {
        let mut session = pending_session(4);
        let mut response = RESPONSE;
        assert_eq!(
            random_response(&mut session, &mut response[..88], 4),
            Err(IoErr::Integrity)
        );
    }

    #[ktest]
    fn limits_random_bytes_to_the_request() {
        let mut session = pending_session(3);
        let mut response = RESPONSE;
        assert_eq!(
            random_response(&mut session, &mut response, 3),
            Ok(&[0xa1, 0xb2, 0xc3][..])
        );
    }

    #[ktest]
    fn rejects_invalid_primary_point() {
        let mut params = Vec::new();
        let public_len = PUBLIC_PREFIX_SIZE + 4 + 2 * SHA256_LEN;
        params.extend_from_slice(&(public_len as u16).to_be_bytes());
        params.extend_from_slice(&NULL_PRIMARY_TEMPLATE[..PUBLIC_PREFIX_SIZE]);
        for _ in 0..2 {
            params.extend_from_slice(&(SHA256_LEN as u16).to_be_bytes());
            params.extend_from_slice(&[0; SHA256_LEN]);
        }
        assert!(matches!(parse_public(&params), Err(IoErr::Protocol)));
    }
    #[ktest]
    fn releases_session_after_a_command_error() {
        use alloc::rc::Rc;
        use core::cell::RefCell;

        struct Transport(Rc<RefCell<Vec<(u32, u32)>>>);
        impl ChipTransport for Transport {
            fn exec(&mut self, request: &[u8], response: &mut [u8]) -> Result<usize, IoErr> {
                let cc = u32::from_be_bytes(request[6..10].try_into().unwrap());
                let handle = if cc == 0x165 {
                    u32::from_be_bytes(request[10..14].try_into().unwrap())
                } else {
                    0
                };
                self.0.borrow_mut().push((cc, handle));
                let packet = match cc {
                    0x161 => command(
                        TpmTag::Tpm2NoSessions.code(),
                        0,
                        &0x8000_0000u32.to_be_bytes(),
                    ),
                    CC_START_AUTH_SESSION => {
                        let mut payload = Vec::new();
                        payload.extend_from_slice(&0x0200_0000u32.to_be_bytes());
                        payload.extend_from_slice(&32u16.to_be_bytes());
                        payload.extend_from_slice(&[0x22; 32]);
                        command(TpmTag::Tpm2NoSessions.code(), 0, &payload)
                    }
                    0x165 => command(TpmTag::Tpm2NoSessions.code(), 0, &[]),
                    CC_GET_RANDOM => command(TpmTag::Tpm2NoSessions.code(), 0x101, &[]),
                    _ => panic!("unexpected TPM command"),
                };
                response[..packet.len()].copy_from_slice(&packet);
                Ok(packet.len())
            }
        }
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut io = CtxIo::new(Transport(calls.clone()));
        let mut scalar = [0u8; 32];
        scalar[31] = 1;
        let mut random = TpmRandom {
            primary: Some(Primary {
                context: Zeroizing::new(vec![0; 18]),
                public_key: SecretKey::from_slice(&scalar).unwrap().public_key(),
            }),
        };
        let mut rng = ExtRng::new(|bytes: &mut [u8]| bytes.fill(0x33));
        let mut out = [0xaa; 4];
        assert_eq!(random.read(&mut io, &mut out, &mut rng), Err(IoErr::Fatal));
        assert_eq!(out, [0xaa; 4]);
        assert_eq!(
            *calls.borrow(),
            [
                (0x161, 0),
                (CC_START_AUTH_SESSION, 0),
                (0x165, 0x8000_0000),
                (CC_GET_RANDOM, 0),
                (0x165, 0x0200_0000),
            ]
        );
    }

    struct RepeatedResponse {
        response: &'static [u8],
        calls: alloc::rc::Rc<core::cell::Cell<usize>>,
    }

    impl ChipTransport for RepeatedResponse {
        fn exec(&mut self, request: &[u8], response: &mut [u8]) -> Result<usize, IoErr> {
            assert_eq!(&request[6..10], &CC_GET_RANDOM.to_be_bytes());
            self.calls.set(self.calls.get() + 1);
            response[..self.response.len()].copy_from_slice(self.response);
            Ok(self.response.len())
        }
    }

    #[ktest]
    fn returns_short_data_after_six_commands() {
        let calls = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let mut io = CtxIo::new(RepeatedResponse {
            response: &RESPONSE,
            calls: calls.clone(),
        });
        let mut session = pending_session(32);
        let mut rng = ExtRng::new(|bytes: &mut [u8]| bytes.fill(0x33));
        let mut out = [0xaa; 32];
        assert_eq!(
            read_session(&mut session, &mut io, &mut out, &mut rng),
            Ok(24)
        );
        assert_eq!(calls.get(), 6);
        assert_eq!(&out[..24], &[0xa1, 0xb2, 0xc3, 0xd4].repeat(6));
        assert_eq!(&out[24..], &[0xaa; 8]);
    }

    #[ktest]
    fn returns_error_after_six_empty_responses() {
        const EMPTY_RESPONSE: [u8; 85] = [
            0x80, 0x02, 0x00, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02,
            0x00, 0x00, 0x00, 0x20, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44,
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44,
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x41, 0x00, 0x20, 0xff, 0xb9, 0xc4,
            0xb5, 0x9e, 0x62, 0xde, 0x1e, 0x4b, 0xea, 0x6b, 0x39, 0x58, 0xe9, 0x62, 0xe3, 0x6b,
            0x17, 0xde, 0x12, 0x47, 0x29, 0x44, 0xd1, 0x8d, 0x20, 0x24, 0x07, 0x64, 0xbf, 0xcc,
            0x7e,
        ];
        let calls = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let mut io = CtxIo::new(RepeatedResponse {
            response: &EMPTY_RESPONSE,
            calls: calls.clone(),
        });
        let mut session = pending_session(32);
        let mut rng = ExtRng::new(|bytes: &mut [u8]| bytes.fill(0x33));
        let mut out = [0xaa; 32];
        assert_eq!(
            read_session(&mut session, &mut io, &mut out, &mut rng),
            Err(IoErr::Fatal)
        );
        assert_eq!(calls.get(), 6);
        assert_eq!(out, [0xaa; 32]);
    }

    #[ktest]
    fn rejects_invalid_requests_before_accessing_the_chip() {
        struct UnreachableTransport;
        impl ChipTransport for UnreachableTransport {
            fn exec(&mut self, _: &[u8], _: &mut [u8]) -> Result<usize, IoErr> {
                panic!("invalid request reached the TPM");
            }
        }
        let mut io = CtxIo::new(UnreachableTransport);
        let mut random = TpmRandom::new();
        let mut rng = ExtRng::new(|bytes: &mut [u8]| bytes.fill(0x33));
        assert_eq!(
            random.read(&mut io, &mut [], &mut rng),
            Err(IoErr::BadCommand)
        );
        assert_eq!(
            random.read(&mut io, &mut [0; 129], &mut rng),
            Err(IoErr::BadCommand)
        );
    }
}

use std::{
    fs,
    io::ErrorKind,
    mem,
    path::{Path, PathBuf},
    str::FromStr,
};

use rustc_ast::AttrId;
use rustc_hash::FxHashMap;
use rustc_hir::def_id::{CrateNum, DefId, DefIndex};
use rustc_middle::{
    mir::interpret::AllocId,
    ty::{PredicateKind, Ty, TyCtxt},
};
use rustc_serialize::{
    opaque::{FileEncoder, MemDecoder},
    Decodable, Decoder, Encodable, Encoder,
};
use rustc_session::StableCrateId;
use rustc_span::{
    hygiene::ExpnIndex, BytePos, ExpnId, FileName, RealFileName, Span, SpanData, SpanDecoder,
    SpanEncoder, StableSourceFileId, Symbol, SyntaxContext, DUMMY_SP,
};
use rustc_type_ir::{TyDecoder, TyEncoder};

// === Entry-points === //

pub fn save_to_file<'tcx, T>(tcx: TyCtxt<'tcx>, name: &str, path: &Path, item: &T)
where
    T: for<'a> Encodable<AutokenEncoder<'tcx, 'a>>,
{
    let encoder = FileEncoder::new(path).unwrap_or_else(|err| {
        tcx.dcx()
            .fatal(format!("failed to serialize {name} to file: {err}"));
    });

    let mut encoder = AutokenEncoder {
        _dummy: [],
        tcx,
        encoder,
        type_shorthands: FxHashMap::default(),
        predicate_shorthands: FxHashMap::default(),
    };

    // Encode set of source files to preload for span generation
    for file in tcx.sess.source_map().files().iter() {
        let FileName::Real(RealFileName::LocalPath(path)) = &file.name else {
            continue;
        };

        let Some(path) = path.to_str() else {
            continue;
        };

        if path.is_empty() {
            continue;
        }

        encoder.emit_str(path);
    }

    encoder.emit_str("");

    // Encode item
    item.encode(&mut encoder);

    if let Err((_, err)) = encoder.encoder.finish() {
        tcx.dcx()
            .fatal(format!("failed to serialize {name} to file: {err}"));
    }
}

pub fn try_load_from_file<'tcx, T>(tcx: TyCtxt<'tcx>, name: &str, path: &Path) -> Option<T>
where
    T: for<'a> Decodable<AutokenDecoder<'tcx, 'a>>,
{
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return None;
        }
        Err(err) => {
            tcx.dcx()
                .fatal(format!("failed to deserialize {name} from file: {err}"));
        }
    };

    let mut decoder = AutokenDecoder {
        tcx,
        decoder: MemDecoder::new(&data, 0),
        ty_cache: FxHashMap::default(),
    };

    // Load preloaded source files
    loop {
        let preload_path = decoder.read_str();
        if preload_path.is_empty() {
            break;
        }

        let _ = tcx.sess.source_map().load_file(Path::new(&preload_path));
    }

    // Load decoded value
    Some(T::decode(&mut decoder))
}

pub fn get_crate_cache_path(tcx: TyCtxt<'_>, krate: CrateNum) -> PathBuf {
    // TODO: Find a better way
    PathBuf::from_str(&format!(
        "{}/autoken_{}_{:x}.meta",
        std::env::var("CARGO_TARGET_DIR").unwrap(),
        tcx.crate_name(krate),
        tcx.stable_crate_id(krate)
    ))
    .unwrap()
}

// === Encoder === //

pub struct AutokenEncoder<'tcx, 'a> {
    _dummy: [&'a (); 0],
    tcx: TyCtxt<'tcx>,
    encoder: FileEncoder,
    type_shorthands: FxHashMap<Ty<'tcx>, usize>,
    predicate_shorthands: FxHashMap<PredicateKind<'tcx>, usize>,
}

impl<'tcx, 'a> TyEncoder for AutokenEncoder<'tcx, 'a> {
    type I = TyCtxt<'tcx>;

    const CLEAR_CROSS_CRATE: bool = true;

    fn position(&self) -> usize {
        self.encoder.position()
    }

    fn type_shorthands(&mut self) -> &mut FxHashMap<Ty<'tcx>, usize> {
        &mut self.type_shorthands
    }

    fn predicate_shorthands(&mut self) -> &mut FxHashMap<PredicateKind<'tcx>, usize> {
        &mut self.predicate_shorthands
    }

    fn encode_alloc_id(&mut self, alloc_id: &AllocId) {
        let _ = alloc_id;
        unimplemented!("not used by analyzer");
    }
}

impl<'tcx, 'a> SpanEncoder for AutokenEncoder<'tcx, 'a> {
    fn encode_span(&mut self, span: Span) {
        let src_file = self.tcx.sess.source_map().lookup_source_file(span.lo());
        let src_offset = src_file.start_pos;

        src_file.stable_id.encode(self);
        (span.lo() - src_offset).encode(self);
        (span.hi() - src_offset).encode(self);
        span.data().ctxt.encode(self);
    }

    fn encode_symbol(&mut self, symbol: Symbol) {
        self.emit_str(symbol.as_str());
    }

    fn encode_expn_id(&mut self, expn_id: ExpnId) {
        expn_id.krate.encode(self);
        expn_id.local_id.as_u32().encode(self);
    }

    fn encode_syntax_context(&mut self, syntax_context: SyntaxContext) {
        // Unimplemented: not used by our analyzer
        let _ = syntax_context;
    }

    fn encode_crate_num(&mut self, crate_num: CrateNum) {
        self.tcx.stable_crate_id(crate_num).encode(self);
    }

    fn encode_def_index(&mut self, def_index: DefIndex) {
        self.emit_u32(def_index.as_u32());
    }

    fn encode_def_id(&mut self, def_id: DefId) {
        def_id.krate.encode(self);
        def_id.index.encode(self);
    }
}

impl<'tcx, 'a> Encoder for AutokenEncoder<'tcx, 'a> {
    fn emit_usize(&mut self, v: usize) {
        self.encoder.emit_usize(v)
    }

    fn emit_u128(&mut self, v: u128) {
        self.encoder.emit_u128(v)
    }

    fn emit_u64(&mut self, v: u64) {
        self.encoder.emit_u64(v)
    }

    fn emit_u32(&mut self, v: u32) {
        self.encoder.emit_u32(v)
    }

    fn emit_u16(&mut self, v: u16) {
        self.encoder.emit_u16(v)
    }

    fn emit_u8(&mut self, v: u8) {
        self.encoder.emit_u8(v)
    }

    fn emit_isize(&mut self, v: isize) {
        self.encoder.emit_isize(v)
    }

    fn emit_i128(&mut self, v: i128) {
        self.encoder.emit_i128(v)
    }

    fn emit_i64(&mut self, v: i64) {
        self.encoder.emit_i64(v)
    }

    fn emit_i32(&mut self, v: i32) {
        self.encoder.emit_i32(v)
    }

    fn emit_i16(&mut self, v: i16) {
        self.encoder.emit_i16(v)
    }

    fn emit_raw_bytes(&mut self, s: &[u8]) {
        self.encoder.emit_raw_bytes(s)
    }
}

// === Decoder === //

pub struct AutokenDecoder<'tcx, 'a> {
    tcx: TyCtxt<'tcx>,
    decoder: MemDecoder<'a>,
    ty_cache: FxHashMap<usize, Ty<'tcx>>,
}

impl<'tcx, 'a> TyDecoder for AutokenDecoder<'tcx, 'a> {
    type I = TyCtxt<'tcx>;

    const CLEAR_CROSS_CRATE: bool = true;

    fn interner(&self) -> Self::I {
        self.tcx
    }

    fn cached_ty_for_shorthand<F>(&mut self, shorthand: usize, or_insert_with: F) -> Ty<'tcx>
    where
        F: FnOnce(&mut Self) -> Ty<'tcx>,
    {
        if let Some(&cached) = self.ty_cache.get(&shorthand) {
            return cached;
        }

        let ty = or_insert_with(self);
        self.ty_cache.insert(shorthand, ty);
        ty
    }

    fn with_position<F, R>(&mut self, pos: usize, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        let new_decoder = MemDecoder::new(self.decoder.data(), pos);
        let old_decoder = mem::replace(&mut self.decoder, new_decoder);
        let r = f(self);
        self.decoder = old_decoder;
        r
    }

    fn decode_alloc_id(&mut self) -> AllocId {
        unimplemented!("not used by analyzer");
    }
}

impl<'tcx, 'a> SpanDecoder for AutokenDecoder<'tcx, 'a> {
    fn decode_span(&mut self) -> Span {
        let src_file = StableSourceFileId::decode(self);
        let rel_lo = BytePos::decode(self);
        let rel_hi = BytePos::decode(self);
        let ctxt = SyntaxContext::decode(self);

        let Some(src_file) = self
            .tcx
            .sess
            .source_map()
            .source_file_by_stable_id(src_file)
        else {
            return DUMMY_SP;
        };

        let file_offset = src_file.start_pos;

        SpanData {
            lo: file_offset + rel_lo,
            hi: file_offset + rel_hi,
            ctxt,
            parent: None,
        }
        .span()
    }

    fn decode_symbol(&mut self) -> Symbol {
        Symbol::intern(self.read_str())
    }

    fn decode_expn_id(&mut self) -> ExpnId {
        let krate = CrateNum::decode(self);
        let local_id = ExpnIndex::from_u32(u32::decode(self));
        ExpnId { krate, local_id }
    }

    fn decode_syntax_context(&mut self) -> SyntaxContext {
        // Unimplemented: not used by our analyzer
        SyntaxContext::root()
    }

    fn decode_crate_num(&mut self) -> CrateNum {
        self.tcx
            .untracked()
            .cstore
            .read()
            .stable_crate_id_to_crate_num(StableCrateId::decode(self))
    }

    fn decode_def_index(&mut self) -> DefIndex {
        DefIndex::from_u32(self.read_u32())
    }

    fn decode_def_id(&mut self) -> DefId {
        let krate = CrateNum::decode(self);
        let index = DefIndex::decode(self);
        DefId { index, krate }
    }

    fn decode_attr_id(&mut self) -> AttrId {
        unimplemented!("not used by analyzer");
    }
}

impl<'tcx, 'a> Decoder for AutokenDecoder<'tcx, 'a> {
    fn read_usize(&mut self) -> usize {
        self.decoder.read_usize()
    }

    fn read_u128(&mut self) -> u128 {
        self.decoder.read_u128()
    }

    fn read_u64(&mut self) -> u64 {
        self.decoder.read_u64()
    }

    fn read_u32(&mut self) -> u32 {
        self.decoder.read_u32()
    }

    fn read_u16(&mut self) -> u16 {
        self.decoder.read_u16()
    }

    fn read_u8(&mut self) -> u8 {
        self.decoder.read_u8()
    }

    fn read_isize(&mut self) -> isize {
        self.decoder.read_isize()
    }

    fn read_i128(&mut self) -> i128 {
        self.decoder.read_i128()
    }

    fn read_i64(&mut self) -> i64 {
        self.decoder.read_i64()
    }

    fn read_i32(&mut self) -> i32 {
        self.decoder.read_i32()
    }

    fn read_i16(&mut self) -> i16 {
        self.decoder.read_i16()
    }

    fn read_raw_bytes(&mut self, len: usize) -> &[u8] {
        self.decoder.read_raw_bytes(len)
    }

    fn peek_byte(&self) -> u8 {
        self.decoder.peek_byte()
    }

    fn position(&self) -> usize {
        self.decoder.position()
    }
}

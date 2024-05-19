use std::{
    collections::hash_map::Entry,
    mem,
    path::{Path, PathBuf},
    sync::Arc,
};

use rustc_ast::AttrId;
use rustc_data_structures::fx::FxIndexSet;
use rustc_hash::FxHashMap;
use rustc_hir::def_id::{CrateNum, DefId, DefIndex, LOCAL_CRATE};
use rustc_macros::{Decodable, Encodable};
use rustc_middle::{
    mir::interpret::AllocId,
    ty::{PredicateKind, Ty, TyCtxt},
};
use rustc_serialize::{
    opaque::{FileEncoder, MemDecoder},
    Decodable, Decoder, Encodable, Encoder,
};
use rustc_span::{
    hygiene::{raw_encode_syntax_context, ExpnIndex, HygieneEncodeContext},
    ExpnId, ExternalSource, SourceFile, Span, SpanData, SpanDecoder, SpanEncoder, Symbol,
    SyntaxContext,
};
use rustc_type_ir::{TyDecoder, TyEncoder};

// === Entry-points === //

pub fn save_to_file<'tcx, T>(tcx: TyCtxt<'tcx>, name: &str, path: &Path, item: &T)
where
    T: for<'a> Encodable<AutokenEncoder<'tcx, 'a>>,
{
    todo!();
}

pub fn try_load_from_file<'tcx, T>(tcx: TyCtxt<'tcx>, name: &str, path: &Path) -> Option<T>
where
    T: for<'a> Decodable<AutokenDecoder<'tcx, 'a>>,
{
    todo!();
}

pub fn get_crate_cache_path(tcx: TyCtxt<'_>, krate: CrateNum) -> PathBuf {
    todo!();
}

// === Helper Formats === //

// Copied from compiler/rustc_metadata/src/rmeta/mod.rs:485

/// A span tag byte encodes a bunch of data, so that we can cut out a few extra bytes from span
/// encodings (which are very common, for example, libcore has ~650,000 unique spans and over 1.1
/// million references to prior-written spans).
///
/// The byte format is split into several parts:
///
/// [ a a a a a c d d ]
///
/// `a` bits represent the span length. We have 5 bits, so we can store lengths up to 30 inline, with
/// an all-1s pattern representing that the length is stored separately.
///
/// `c` represents whether the span context is zero (and then it is not stored as a separate varint)
/// for direct span encodings, and whether the offset is absolute or relative otherwise (zero for
/// absolute).
///
/// d bits represent the kind of span we are storing (local, foreign, partial, indirect).
#[derive(Encodable, Decodable, Copy, Clone)]
struct SpanTag(u8);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum SpanKind {
    Local = 0b00,
    Foreign = 0b01,
    Partial = 0b10,
    // Indicates the actual span contents are elsewhere.
    // If this is the kind, then the span context bit represents whether it is a relative or
    // absolute offset.
    Indirect = 0b11,
}

impl SpanTag {
    fn new(kind: SpanKind, context: rustc_span::SyntaxContext, length: usize) -> SpanTag {
        let mut data = 0u8;
        data |= kind as u8;
        if context.is_root() {
            data |= 0b100;
        }
        let all_1s_len = (0xffu8 << 3) >> 3;
        // strictly less than - all 1s pattern is a sentinel for storage being out of band.
        if length < all_1s_len as usize {
            data |= (length as u8) << 3;
        } else {
            data |= all_1s_len << 3;
        }

        SpanTag(data)
    }

    fn indirect(relative: bool, length_bytes: u8) -> SpanTag {
        let mut tag = SpanTag(SpanKind::Indirect as u8);
        if relative {
            tag.0 |= 0b100;
        }
        assert!(length_bytes <= 8);
        tag.0 |= length_bytes << 3;
        tag
    }

    fn kind(self) -> SpanKind {
        let masked = self.0 & 0b11;
        match masked {
            0b00 => SpanKind::Local,
            0b01 => SpanKind::Foreign,
            0b10 => SpanKind::Partial,
            0b11 => SpanKind::Indirect,
            _ => unreachable!(),
        }
    }

    fn is_relative_offset(self) -> bool {
        debug_assert_eq!(self.kind(), SpanKind::Indirect);
        self.0 & 0b100 != 0
    }

    fn context(self) -> Option<rustc_span::SyntaxContext> {
        if self.0 & 0b100 != 0 {
            Some(rustc_span::SyntaxContext::root())
        } else {
            None
        }
    }

    fn length(self) -> Option<rustc_span::BytePos> {
        let all_1s_len = (0xffu8 << 3) >> 3;
        let len = self.0 >> 3;
        if len != all_1s_len {
            Some(rustc_span::BytePos(u32::from(len)))
        } else {
            None
        }
    }
}

// Tags for encoding Symbol's
const SYMBOL_STR: u8 = 0;
const SYMBOL_OFFSET: u8 = 1;
const SYMBOL_PREINTERNED: u8 = 2;

// Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:227
impl<'tcx, 'a> Encodable<AutokenEncoder<'tcx, 'a>> for SpanData {
    fn encode(&self, s: &mut AutokenEncoder<'tcx, 'a>) {
        // Don't serialize any `SyntaxContext`s from a proc-macro crate,
        // since we don't load proc-macro dependencies during serialization.
        // This means that any hygiene information from macros used *within*
        // a proc-macro crate (e.g. invoking a macro that expands to a proc-macro
        // definition) will be lost.
        //
        // This can show up in two ways:
        //
        // 1. Any hygiene information associated with identifier of
        // a proc macro (e.g. `#[proc_macro] pub fn $name`) will be lost.
        // Since proc-macros can only be invoked from a different crate,
        // real code should never need to care about this.
        //
        // 2. Using `Span::def_site` or `Span::mixed_site` will not
        // include any hygiene information associated with the definition
        // site. This means that a proc-macro cannot emit a `$crate`
        // identifier which resolves to one of its dependencies,
        // which also should never come up in practice.
        //
        // Additionally, this affects `Span::parent`, and any other
        // span inspection APIs that would otherwise allow traversing
        // the `SyntaxContexts` associated with a span.
        //
        // None of these user-visible effects should result in any
        // cross-crate inconsistencies (getting one behavior in the same
        // crate, and a different behavior in another crate) due to the
        // limited surface that proc-macros can expose.
        //
        // IMPORTANT: If this is ever changed, be sure to update
        // `rustc_span::hygiene::raw_encode_expn_id` to handle
        // encoding `ExpnData` for proc-macro crates.
        let ctxt = if s.is_proc_macro {
            SyntaxContext::root()
        } else {
            self.ctxt
        };

        if self.is_dummy() {
            let tag = SpanTag::new(SpanKind::Partial, ctxt, 0);
            tag.encode(s);
            if tag.context().is_none() {
                ctxt.encode(s);
            }
            return;
        }

        // The Span infrastructure should make sure that this invariant holds:
        debug_assert!(self.lo <= self.hi);

        if !s.source_file_cache.0.contains(self.lo) {
            let source_map = s.tcx.sess.source_map();
            let source_file_index = source_map.lookup_source_file_idx(self.lo);
            s.source_file_cache = (
                source_map.files()[source_file_index].clone(),
                source_file_index,
            );
        }
        let (ref source_file, source_file_index) = s.source_file_cache;
        debug_assert!(source_file.contains(self.lo));

        if !source_file.contains(self.hi) {
            // Unfortunately, macro expansion still sometimes generates Spans
            // that malformed in this way.
            let tag = SpanTag::new(SpanKind::Partial, ctxt, 0);
            tag.encode(s);
            if tag.context().is_none() {
                ctxt.encode(s);
            }
            return;
        }

        // There are two possible cases here:
        // 1. This span comes from a 'foreign' crate - e.g. some crate upstream of the
        // crate we are writing metadata for. When the metadata for *this* crate gets
        // deserialized, the deserializer will need to know which crate it originally came
        // from. We use `TAG_VALID_SPAN_FOREIGN` to indicate that a `CrateNum` should
        // be deserialized after the rest of the span data, which tells the deserializer
        // which crate contains the source map information.
        // 2. This span comes from our own crate. No special handling is needed - we just
        // write `TAG_VALID_SPAN_LOCAL` to let the deserializer know that it should use
        // our own source map information.
        //
        // If we're a proc-macro crate, we always treat this as a local `Span`.
        // In `encode_source_map`, we serialize foreign `SourceFile`s into our metadata
        // if we're a proc-macro crate.
        // This allows us to avoid loading the dependencies of proc-macro crates: all of
        // the information we need to decode `Span`s is stored in the proc-macro crate.
        let (kind, metadata_index) = if source_file.is_imported() && !s.is_proc_macro {
            // To simplify deserialization, we 'rebase' this span onto the crate it originally came
            // from (the crate that 'owns' the file it references. These rebased 'lo' and 'hi'
            // values are relative to the source map information for the 'foreign' crate whose
            // CrateNum we write into the metadata. This allows `imported_source_files` to binary
            // search through the 'foreign' crate's source map information, using the
            // deserialized 'lo' and 'hi' values directly.
            //
            // All of this logic ensures that the final result of deserialization is a 'normal'
            // Span that can be used without any additional trouble.
            let metadata_index = {
                // Introduce a new scope so that we drop the 'read()' temporary
                match &*source_file.external_src.read() {
                    ExternalSource::Foreign { metadata_index, .. } => *metadata_index,
                    src => panic!("Unexpected external source {src:?}"),
                }
            };

            (SpanKind::Foreign, metadata_index)
        } else {
            // Record the fact that we need to encode the data for this `SourceFile`
            let source_files = s
                .required_source_files
                .as_mut()
                .expect("Already encoded SourceMap!");
            let (metadata_index, _) = source_files.insert_full(source_file_index);
            let metadata_index: u32 = metadata_index
                .try_into()
                .expect("cannot export more than U32_MAX files");

            (SpanKind::Local, metadata_index)
        };

        // Encode the start position relative to the file start, so we profit more from the
        // variable-length integer encoding.
        let lo = self.lo - source_file.start_pos;

        // Encode length which is usually less than span.hi and profits more
        // from the variable-length integer encoding that we use.
        let len = self.hi - self.lo;

        let tag = SpanTag::new(kind, ctxt, len.0 as usize);
        tag.encode(s);
        if tag.context().is_none() {
            ctxt.encode(s);
        }
        lo.encode(s);
        if tag.length().is_none() {
            len.encode(s);
        }

        // Encode the index of the `SourceFile` for the span, in order to make decoding faster.
        metadata_index.encode(s);

        if kind == SpanKind::Foreign {
            // This needs to be two lines to avoid holding the `s.source_file_cache`
            // while calling `cnum.encode(s)`
            let cnum = s.source_file_cache.0.cnum;
            cnum.encode(s);
        }
    }
}

// Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:126
impl<'tcx, 'a> Encodable<AutokenEncoder<'tcx, 'a>> for ExpnIndex {
    fn encode(&self, s: &mut AutokenEncoder<'tcx, 'a>) {
        s.emit_u32(self.as_u32());
    }
}

// === Encoder === //

pub struct AutokenEncoder<'tcx, 'a> {
    tcx: TyCtxt<'tcx>,
    encoder: FileEncoder,
    is_proc_macro: bool,

    //> Crate data:
    type_shorthands: FxHashMap<Ty<'tcx>, usize>,
    predicate_shorthands: FxHashMap<PredicateKind<'tcx>, usize>,
    interpret_allocs: FxIndexSet<AllocId>,
    span_shorthands: FxHashMap<Span, usize>,
    hygiene_ctxt: &'a HygieneEncodeContext,

    // The indices (into the `SourceMap`'s `MonotonicVec`)
    // of all of the `SourceFiles` that we need to serialize.
    // When we serialize a `Span`, we insert the index of its
    // `SourceFile` into the `FxIndexSet`.
    // The order inside the `FxIndexSet` is used as on-disk
    // order of `SourceFiles`, and encoded inside `Span`s.
    required_source_files: Option<FxIndexSet<usize>>,

    //> Procedural builder state:
    symbol_table: FxHashMap<Symbol, usize>,

    //> Caches:

    // This is used to speed up Span encoding.
    // The `usize` is an index into the `MonotonicVec`
    // that stores the `SourceFile`
    source_file_cache: (Arc<SourceFile>, usize),
}

impl<'tcx, 'a> TyEncoder for AutokenEncoder<'tcx, 'a> {
    type I = TyCtxt<'tcx>;

    const CLEAR_CROSS_CRATE: bool = true;

    // Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:382
    fn position(&self) -> usize {
        self.encoder.position()
    }

    // Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:386
    fn type_shorthands(&mut self) -> &mut FxHashMap<Ty<'tcx>, usize> {
        &mut self.type_shorthands
    }

    // Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:390
    fn predicate_shorthands(&mut self) -> &mut FxHashMap<PredicateKind<'tcx>, usize> {
        &mut self.predicate_shorthands
    }

    // Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:394
    fn encode_alloc_id(&mut self, alloc_id: &AllocId) {
        let (index, _) = self.interpret_allocs.insert_full(*alloc_id);

        index.encode(self);
    }
}

impl<'tcx, 'a> SpanEncoder for AutokenEncoder<'tcx, 'a> {
    // Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:165
    fn encode_span(&mut self, span: Span) {
        fn bytes_needed(n: usize) -> usize {
            (usize::BITS - n.leading_zeros()).div_ceil(u8::BITS) as usize
        }

        match self.span_shorthands.entry(span) {
            Entry::Occupied(o) => {
                // If an offset is smaller than the absolute position, we encode with the offset.
                // This saves space since smaller numbers encode in less bits.
                let last_location = *o.get();
                // This cannot underflow. Metadata is written with increasing position(), so any
                // previously saved offset must be smaller than the current position.
                let offset = self.encoder.position() - last_location;
                if offset < last_location {
                    let needed = bytes_needed(offset);
                    SpanTag::indirect(true, needed as u8).encode(self);
                    self.encoder.write_with(|dest| {
                        *dest = offset.to_le_bytes();
                        needed
                    });
                } else {
                    let needed = bytes_needed(last_location);
                    SpanTag::indirect(false, needed as u8).encode(self);
                    self.encoder.write_with(|dest| {
                        *dest = last_location.to_le_bytes();
                        needed
                    });
                }
            }
            Entry::Vacant(v) => {
                let position = self.encoder.position();
                v.insert(position);
                // Data is encoded with a SpanTag prefix (see below).
                span.data().encode(self);
            }
        }
    }

    // Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:199
    fn encode_symbol(&mut self, symbol: Symbol) {
        // if symbol preinterned, emit tag and symbol index
        if symbol.is_preinterned() {
            self.encoder.emit_u8(SYMBOL_PREINTERNED);
            self.encoder.emit_u32(symbol.as_u32());
        } else {
            // otherwise write it as string or as offset to it
            match self.symbol_table.entry(symbol) {
                Entry::Vacant(o) => {
                    self.encoder.emit_u8(SYMBOL_STR);
                    let pos = self.encoder.position();
                    o.insert(pos);
                    self.emit_str(symbol.as_str());
                }
                Entry::Occupied(o) => {
                    let x = *o.get();
                    self.emit_u8(SYMBOL_OFFSET);
                    self.emit_usize(x);
                }
            }
        }
    }

    // Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:153
    fn encode_expn_id(&mut self, expn_id: ExpnId) {
        if expn_id.krate == LOCAL_CRATE {
            // We will only write details for local expansions. Non-local expansions will fetch
            // data from the corresponding crate's metadata.
            self.hygiene_ctxt.schedule_expn_data_for_encoding(expn_id);
        }
        expn_id.krate.encode(self);
        expn_id.local_id.encode(self);
    }

    // Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:149
    fn encode_syntax_context(&mut self, syntax_context: SyntaxContext) {
        raw_encode_syntax_context(syntax_context, self.hygiene_ctxt, self);
    }

    // Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:133
    fn encode_crate_num(&mut self, crate_num: CrateNum) {
        if crate_num != LOCAL_CRATE && self.is_proc_macro {
            panic!("Attempted to encode non-local CrateNum {crate_num:?} for proc-macro crate");
        }
        self.emit_u32(crate_num.as_u32());
    }

    // Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:140
    fn encode_def_index(&mut self, def_index: DefIndex) {
        self.emit_u32(def_index.as_u32());
    }

    // Copied from compiler/rustc_metadata/src/rmeta/encoder.rs:144
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
        todo!()
    }

    // Copied from compiler/rustc_metadata/src/rmeta/decoder.rs:391
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
        todo!()
    }
}

impl<'tcx, 'a> SpanDecoder for AutokenDecoder<'tcx, 'a> {
    fn decode_span(&mut self) -> Span {
        todo!()
    }

    // Copied from compiler/rustc_metadata/src/rmeta/decoder.rs:524
    fn decode_symbol(&mut self) -> Symbol {
        let tag = self.read_u8();

        match tag {
            SYMBOL_STR => {
                let s = self.read_str();
                Symbol::intern(s)
            }
            SYMBOL_OFFSET => {
                // read str offset
                let pos = self.read_usize();

                // move to str offset and read
                self.decoder.with_position(pos, |d| {
                    let s = d.read_str();
                    Symbol::intern(s)
                })
            }
            SYMBOL_PREINTERNED => {
                let symbol_index = self.read_u32();
                Symbol::new_from_decoded(symbol_index)
            }
            _ => unreachable!(),
        }
    }

    fn decode_expn_id(&mut self) -> ExpnId {
        todo!()
    }

    fn decode_syntax_context(&mut self) -> SyntaxContext {
        todo!()
    }

    fn decode_crate_num(&mut self) -> CrateNum {
        todo!()
    }

    fn decode_def_index(&mut self) -> DefIndex {
        todo!()
    }

    fn decode_def_id(&mut self) -> DefId {
        todo!()
    }

    fn decode_attr_id(&mut self) -> AttrId {
        todo!()
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

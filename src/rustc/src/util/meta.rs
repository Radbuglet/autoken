use std::path::{Path, PathBuf};

use rustc_ast::AttrId;
use rustc_hash::FxHashMap;
use rustc_hir::def_id::{CrateNum, DefId, DefIndex};
use rustc_middle::{
    mir::interpret::AllocId,
    ty::{PredicateKind, Ty, TyCtxt},
};
use rustc_serialize::{Decodable, Decoder, Encodable, Encoder};
use rustc_span::{ExpnId, Span, SpanDecoder, SpanEncoder, Symbol, SyntaxContext};
use rustc_type_ir::{TyDecoder, TyEncoder};

// === Entry-points === //

pub fn save_to_file<'tcx, T>(tcx: TyCtxt<'tcx>, name: &str, path: &Path, item: &T)
where
    T: for<'a> Encodable<AutokenEncoder<'tcx>>,
{
    todo!();
}

pub fn try_load_from_file<'tcx, T>(tcx: TyCtxt<'tcx>, name: &str, path: &Path) -> Option<T>
where
    T: for<'a> Decodable<AutokenDecoder<'tcx>>,
{
    todo!();
}

pub fn get_crate_cache_path(tcx: TyCtxt<'_>, krate: CrateNum) -> PathBuf {
    todo!();
}

// === Encoder === //

pub struct AutokenEncoder<'tcx> {
    tcx: TyCtxt<'tcx>,
}

impl<'tcx> TyEncoder for AutokenEncoder<'tcx> {
    type I = TyCtxt<'tcx>;

    const CLEAR_CROSS_CRATE: bool = true;

    fn position(&self) -> usize {
        todo!()
    }

    fn type_shorthands(&mut self) -> &mut FxHashMap<Ty<'tcx>, usize> {
        todo!()
    }

    fn predicate_shorthands(&mut self) -> &mut FxHashMap<PredicateKind<'tcx>, usize> {
        todo!()
    }

    fn encode_alloc_id(&mut self, alloc_id: &AllocId) {
        todo!()
    }
}

impl<'tcx> SpanEncoder for AutokenEncoder<'tcx> {
    fn encode_span(&mut self, span: Span) {
        todo!()
    }

    fn encode_symbol(&mut self, symbol: Symbol) {
        todo!()
    }

    fn encode_expn_id(&mut self, expn_id: ExpnId) {
        todo!()
    }

    fn encode_syntax_context(&mut self, syntax_context: SyntaxContext) {
        todo!()
    }

    fn encode_crate_num(&mut self, crate_num: CrateNum) {
        todo!()
    }

    fn encode_def_index(&mut self, def_index: DefIndex) {
        todo!()
    }

    fn encode_def_id(&mut self, def_id: DefId) {
        todo!()
    }
}

impl<'tcx> Encoder for AutokenEncoder<'tcx> {
    fn emit_usize(&mut self, v: usize) {
        todo!()
    }

    fn emit_u128(&mut self, v: u128) {
        todo!()
    }

    fn emit_u64(&mut self, v: u64) {
        todo!()
    }

    fn emit_u32(&mut self, v: u32) {
        todo!()
    }

    fn emit_u16(&mut self, v: u16) {
        todo!()
    }

    fn emit_u8(&mut self, v: u8) {
        todo!()
    }

    fn emit_isize(&mut self, v: isize) {
        todo!()
    }

    fn emit_i128(&mut self, v: i128) {
        todo!()
    }

    fn emit_i64(&mut self, v: i64) {
        todo!()
    }

    fn emit_i32(&mut self, v: i32) {
        todo!()
    }

    fn emit_i16(&mut self, v: i16) {
        todo!()
    }

    fn emit_raw_bytes(&mut self, s: &[u8]) {
        todo!()
    }
}

// === Decoder === //

pub struct AutokenDecoder<'tcx> {
    tcx: TyCtxt<'tcx>,
}

impl<'tcx> TyDecoder for AutokenDecoder<'tcx> {
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

    fn with_position<F, R>(&mut self, pos: usize, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        todo!()
    }

    fn decode_alloc_id(&mut self) -> AllocId {
        todo!()
    }
}

impl<'tcx> SpanDecoder for AutokenDecoder<'tcx> {
    fn decode_span(&mut self) -> Span {
        todo!()
    }

    fn decode_symbol(&mut self) -> Symbol {
        todo!()
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

impl<'tcx> Decoder for AutokenDecoder<'tcx> {
    fn read_usize(&mut self) -> usize {
        todo!()
    }

    fn read_u128(&mut self) -> u128 {
        todo!()
    }

    fn read_u64(&mut self) -> u64 {
        todo!()
    }

    fn read_u32(&mut self) -> u32 {
        todo!()
    }

    fn read_u16(&mut self) -> u16 {
        todo!()
    }

    fn read_u8(&mut self) -> u8 {
        todo!()
    }

    fn read_isize(&mut self) -> isize {
        todo!()
    }

    fn read_i128(&mut self) -> i128 {
        todo!()
    }

    fn read_i64(&mut self) -> i64 {
        todo!()
    }

    fn read_i32(&mut self) -> i32 {
        todo!()
    }

    fn read_i16(&mut self) -> i16 {
        todo!()
    }

    fn read_raw_bytes(&mut self, len: usize) -> &[u8] {
        todo!()
    }

    fn peek_byte(&self) -> u8 {
        todo!()
    }

    fn position(&self) -> usize {
        todo!()
    }
}

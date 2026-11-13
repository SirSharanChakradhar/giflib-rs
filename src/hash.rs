//! Exported hash-table helpers from upstream's `gif_hash.c` (ABI compatibility).

#![allow(non_snake_case, clippy::missing_safety_doc)]

use crate::gifalloc::{malloc, slice, slice_mut};
use crate::lzw::HashTable;
use core::ffi::c_int;

const HT_SIZE: usize = 8192;

/// `GifHashTableType` is `struct { uint32_t HTable[8192]; }`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _InitHashTable() -> *mut u32 {
    unsafe {
        let table = malloc(HT_SIZE * 4) as *mut u32;
        if !table.is_null() {
            _ClearHashTable(table);
        }
        table
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ClearHashTable(HashTable: *mut u32) {
    unsafe { slice_mut(HashTable, HT_SIZE).fill(u32::MAX) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _InsertHashTable(Table: *mut u32, Key: u32, Code: c_int) {
    unsafe { HashTable::insert_in(slice_mut(Table, HT_SIZE), Key, Code) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ExistsHashTable(Table: *mut u32, Key: u32) -> c_int {
    unsafe { HashTable::exists_in(slice(Table, HT_SIZE), Key) }
}

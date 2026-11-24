//! C ABI for wasm32: load the blob once, then drive any number of players.
//!   pp_alloc(len) -> ptr; copy the blob there; pp_init(ptr, len) -> 1 on success
//!   pp_player_new() -> handle; pp_play_bgm / pp_play_se / pp_stop / pp_set_volume
//!   pp_render(handle, frames, rate) -> ptr to frames*2 interleaved f32
use crate::{Player, Rom};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static ROM: RefCell<Option<Rc<Rom>>> = const { RefCell::new(None) };
    static PLAYERS: RefCell<Vec<Player>> = const { RefCell::new(Vec::new()) };
}

fn with<T>(handle: u32, f: impl FnOnce(&mut Player) -> T) -> Option<T> {
    PLAYERS.with(|p| p.borrow_mut().get_mut(handle as usize).map(f))
}

#[unsafe(no_mangle)]
pub extern "C" fn pp_alloc(len: usize) -> *mut u8 {
    let mut buffer = std::mem::ManuallyDrop::new(vec![0u8; len]);
    buffer.as_mut_ptr()
}

/// # Safety
/// `ptr` must come from `pp_alloc(len)`; it is taken over and freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_init(ptr: *mut u8, len: usize) -> i32 {
    let blob = unsafe { Vec::from_raw_parts(ptr, len, len) };
    let rom = Rom::from_blob(&blob);
    let ok = rom.is_some() as i32;
    ROM.with(|r| *r.borrow_mut() = rom.map(Rc::new));
    ok
}

#[unsafe(no_mangle)]
pub extern "C" fn pp_player_new() -> i32 {
    let Some(rom) = ROM.with(|r| r.borrow().clone()) else {
        return -1;
    };
    PLAYERS.with(|p| {
        let mut p = p.borrow_mut();
        p.push(Player::new(rom));
        p.len() as i32 - 1
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn pp_play_bgm(handle: u32, id: u32) {
    with(handle, |p| p.play_bgm(id));
}
#[unsafe(no_mangle)]
pub extern "C" fn pp_play_se(handle: u32, id: u32) {
    with(handle, |p| p.play_se(id));
}
#[unsafe(no_mangle)]
pub extern "C" fn pp_stop(handle: u32) {
    with(handle, Player::stop);
}
#[unsafe(no_mangle)]
pub extern "C" fn pp_set_volume(handle: u32, volume: f32) {
    with(handle, |p| p.set_volume(volume));
}
#[unsafe(no_mangle)]
pub extern "C" fn pp_active(handle: u32) -> i32 {
    with(handle, |p| p.active() as i32).unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "C" fn pp_render(handle: u32, frames: usize, rate: u32) -> *const f32 {
    with(handle, |p| {
        p.render(frames, rate);
        p.out.as_ptr()
    })
    .unwrap_or(std::ptr::null())
}

//! Write Pylon realtime shard logic in Rust and compile it to WebAssembly.
//!
//! A Pylon app declares a shard kind in `app.ts` and points it at a `.wasm`
//! file. The stock `pylon` binary loads the module and runs it on the
//! shard's tick thread: it queues inputs, calls `tick` at the shard's rate,
//! and sends each subscriber its snapshot.
//!
//! ```ignore
//! use pylon_shard_guest::{export_shard, Auth, Shard};
//! use serde::{Deserialize, Serialize};
//! use std::time::Duration;
//!
//! #[derive(Default)]
//! struct Arena { players: Vec<Player> }
//!
//! #[derive(Serialize, Clone)]
//! struct Player { id: String, x: f32, y: f32 }
//!
//! #[derive(Deserialize)]
//! struct Move { dx: f32, dy: f32 }
//!
//! impl Shard for Arena {
//!     type Params = serde_json::Value;
//!     type Input = Move;
//!     type Snapshot = Vec<Player>;
//!
//!     fn init(_shard_id: &str, _params: Self::Params) -> Result<Self, String> {
//!         Ok(Arena::default())
//!     }
//!     fn apply_input(&mut self, sid: &str, m: Move) -> Result<(), String> {
//!         // ...
//!         Ok(())
//!     }
//!     fn tick(&mut self, _dt: Duration) {}
//!     fn snapshot(&self) -> Vec<Player> { self.players.clone() }
//! }
//!
//! export_shard!(Arena);
//! ```
//!
//! Build it with `cargo build --release --target wasm32-unknown-unknown`
//! and a `[lib] crate-type = ["cdylib"]` section in `Cargo.toml`.
//!
//! The module has no clock, no randomness source, and no I/O. Put a seed in
//! the shard's params when the simulation needs random numbers, so a replay
//! of the same inputs gives the same result.
//!
//! # ABI (version 1)
//!
//! [`export_shard!`] generates these exports. A module in another language
//! can implement them directly. All integers are `i32` unless noted;
//! pointers and lengths refer to the module's own memory.
//!
//! | Export | Meaning |
//! | --- | --- |
//! | `memory` | The module's linear memory. |
//! | `pylon_shard_abi() -> i32` | Returns `1`. |
//! | `pylon_scratch(len) -> ptr` | A buffer of at least `len` bytes. The host writes call arguments there. It stays valid until the next `pylon_scratch` call. |
//! | `pylon_output_ptr() -> ptr`, `pylon_output_len() -> len` | The bytes the last call produced: a snapshot, or a UTF-8 error message. |
//! | `pylon_init(codec, ptr, len) -> status` | Create the state. `codec` is `0` JSON or `1` MessagePack. The bytes are JSON `{"shard": id, "params": value}`. |
//! | `pylon_apply_input(sid_ptr, sid_len, in_ptr, in_len) -> status` | Apply one input, encoded in the codec. |
//! | `pylon_tick(dt_nanos: i64)` | Advance time. |
//! | `pylon_snapshot() -> status` | The broadcast snapshot, in the codec, to the output. |
//! | `pylon_snapshot_for(sid_ptr, sid_len) -> status` | One subscriber's snapshot. Status `2` means "same as the broadcast snapshot". |
//! | `pylon_is_finished() -> i32` | `1` when the shard should stop. |
//! | `pylon_authorize_subscribe(sid_ptr, sid_len, auth_ptr, auth_len) -> status` | Admit or refuse a subscriber. The auth bytes are JSON ([`Auth`]). |
//! | `pylon_authorize_input(sid_ptr, sid_len, auth_ptr, auth_len, in_ptr, in_len) -> status` | Admit or refuse an input before it is queued. |
//!
//!
//! Optional exports for interest management. A module exports all five or
//! none; `export_shard!` exports all five. Ids are u64 and floats f32, all
//! little-endian.
//!
//! | Export | Meaning |
//! | --- | --- |
//! | `pylon_interest() -> i32` | `0` off. `1` on, output: cell size, margin, flags (u32, bit 0: shared snapshots). |
//! | `pylon_entities() -> status` | Output: 16 bytes per entity (id, x, y). |
//! | `pylon_interest_area(sid_ptr, sid_len) -> i32` | `0` none. `1`, output: x, y, radius. |
//! | `pylon_filter_visible(sid_ptr, sid_len, ids_ptr, ids_len) -> status` | Output: the ids the subscriber may see, in order. |
//! | `pylon_snapshot_visible(sid_ptr, sid_len, view_ptr, view_len) -> status` | The view is three u32 counts (visible, entered, left) and then the ids. Status `2` means "send `snapshot_for`". |
//!
//!
//! Optional export for entity replication:
//!
//! | Export | Meaning |
//! | --- | --- |
//! | `pylon_replication() -> i32` | `0` off. `1` on, output: precision (f32), byte budget (u32), flags (u8: bit 0 x/z plane, bit 1 full dump), then a `pylon_replication` change log. The first call sends a full dump; later calls send the changes since the previous call. |
//!
//! Status `0` is success. Status `1` is an error or a refusal, with a UTF-8
//! message in the output. A trap stops the shard.
//!
//! The one import is `pylon.log(level: i32, ptr, len)`, levels `0` debug to
//! `3` error. The host refuses a module with any other import.

use std::cell::UnsafeCell;
use std::time::Duration;

pub use pylon_replication::{ComponentId, Entity, EntityId, Replicated};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The ABI version [`export_shard!`] implements.
pub const ABI_VERSION: i32 = 1;

/// Who is subscribing or sending an input.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Auth {
    pub user_id: Option<String>,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default)]
    pub roles: Vec<String>,
    pub tenant_id: Option<String>,
    /// A shard ticket the client presented. The host has already checked
    /// its signature, its expiry, and that it names this shard and this
    /// subscriber.
    pub ticket: Option<Ticket>,
}

impl Auth {
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// One app claim from the ticket, by key.
    pub fn claim(&self, key: &str) -> Option<&serde_json::Value> {
        self.ticket.as_ref().and_then(|t| t.claims.get(key))
    }
}

/// A verified shard ticket, minted by `ctx.shards.ticket` in a function.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Ticket {
    pub shard: String,
    pub sid: String,
    pub user_id: Option<String>,
    /// Expiry, in seconds since the Unix epoch.
    pub exp: u64,
    #[serde(default)]
    pub claims: serde_json::Value,
}

/// Shard logic. See the crate docs for an example.
pub trait Shard: Sized + 'static {
    /// The `params` passed to `ctx.shards.create`. Always JSON.
    type Params: DeserializeOwned;
    /// One client input, in the shard's codec.
    type Input: DeserializeOwned;
    /// What subscribers receive every tick, in the shard's codec.
    type Snapshot: Serialize;

    fn init(shard_id: &str, params: Self::Params) -> Result<Self, String>;

    /// Apply one input. Called on the tick, in arrival order. An error goes
    /// back to the sender as an `apply_failed` rejection.
    fn apply_input(&mut self, subscriber: &str, input: Self::Input) -> Result<(), String>;

    /// Advance time by `dt`: the shard's fixed step, or the measured time
    /// for an event-driven shard.
    fn tick(&mut self, dt: Duration);

    /// The snapshot every subscriber gets, unless [`Shard::snapshot_for`]
    /// returns one for them.
    fn snapshot(&self) -> Self::Snapshot;

    /// A snapshot for one subscriber (area of interest, fog of war).
    /// `None` sends the broadcast snapshot.
    fn snapshot_for(&self, _subscriber: &str) -> Option<Self::Snapshot> {
        None
    }

    /// Return true to stop the shard (the match ended).
    fn is_finished(&self) -> bool {
        false
    }

    /// Admit or refuse a subscriber. The default admits an admin, a holder
    /// of a ticket, and a signed-in user whose id is the subscriber id.
    fn authorize_subscribe(&self, subscriber: &str, auth: &Auth) -> Result<(), String> {
        default_authorize_subscribe(subscriber, auth)
    }

    /// Admit or refuse an input before it is queued. The default admits all.
    fn authorize_input(
        &self,
        _subscriber: &str,
        _auth: &Auth,
        _input: &Self::Input,
    ) -> Result<(), String> {
        Ok(())
    }

    // -- Interest management (optional) -----------------------------------
    //
    // Return a config from `interest` and the host computes, each tick,
    // which entities each subscriber sees: it puts `entities` in a grid,
    // keeps those within the subscriber's `interest_area` (with a margin so
    // edge entities do not flicker), drops the ones `can_see` refuses, and
    // asks `snapshot_visible` for the subscriber's snapshot.

    /// Turn interest management on. Default: off.
    fn interest(&self) -> Option<InterestConfig> {
        None
    }

    /// This tick's entity positions.
    fn entities(&self, _out: &mut Vec<EntityPos>) {}

    /// Where a subscriber looks from. `None` sees no entities.
    fn interest_area(&self, _subscriber: &str) -> Option<InterestArea> {
        None
    }

    /// Hide an entity in range from a subscriber (stealth, fog of war).
    fn can_see(&self, _subscriber: &str, _entity: u64) -> bool {
        true
    }

    /// The subscriber's snapshot, given its view. `None` sends what
    /// `snapshot_for` gives.
    fn snapshot_visible(&self, _subscriber: &str, _view: &View<'_>) -> Option<Self::Snapshot> {
        None
    }

    // -- Entity replication (optional) ---------------------------------------
    //
    // Keep entities in a `Replicated` store and return it here: the host
    // then sends each subscriber spawn, update, and despawn frames for the
    // entities in its view (all of them, or the interest view), with only
    // what changed. `snapshot*` is not used then.

    /// The entity store to replicate. Default: none (send snapshots).
    fn replicated(&mut self) -> Option<&mut Replicated> {
        None
    }

    /// Settings for the replication frames.
    fn replication_config(&self) -> ReplicationConfig {
        ReplicationConfig::default()
    }
}

/// Replication settings (see `Shard::replicated`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReplicationConfig {
    /// Position precision in world units.
    pub precision: f32,
    /// Byte budget per subscriber per tick for updates. 0 = no limit.
    pub max_bytes_per_tick: u32,
    /// The ground plane interest management uses: false = x/y, true = x/z.
    pub y_up: bool,
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        Self {
            precision: 0.01,
            max_bytes_per_tick: 0,
            y_up: false,
        }
    }
}

/// Grid settings for interest management.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterestConfig {
    /// Side of a grid cell, in world units. About the view radius works well.
    pub cell_size: f32,
    /// How far past the radius a visible entity may go before it leaves view.
    pub margin: f32,
    /// True when `snapshot_visible` depends only on `view.visible`:
    /// subscribers that see the same entities then share one snapshot.
    pub shared_snapshots: bool,
}

/// One entity's position this tick (2D; a 3D world passes its ground plane).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityPos {
    pub id: u64,
    pub x: f32,
    pub y: f32,
}

/// What a subscriber can see: a circle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterestArea {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
}

/// A subscriber's view this tick. Every list is sorted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View<'a> {
    pub visible: &'a [u64],
    /// Visible now and not last tick.
    pub entered: &'a [u64],
    /// Visible last tick and not now.
    pub left: &'a [u64],
}

/// The host's default subscribe rule.
pub fn default_authorize_subscribe(subscriber: &str, auth: &Auth) -> Result<(), String> {
    if auth.is_admin || auth.ticket.is_some() {
        return Ok(());
    }
    match &auth.user_id {
        Some(uid) if uid == subscriber => Ok(()),
        Some(_) => Err(format!(
            "subscriber id \"{subscriber}\" does not match authenticated user"
        )),
        None => Err("authenticated user required".into()),
    }
}

/// Log levels for [`log`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

/// Write a line to the Pylon server log. The host limits how many lines a
/// shard may write per second.
pub fn log(level: Level, message: &str) {
    #[cfg(target_family = "wasm")]
    {
        #[link(wasm_import_module = "pylon")]
        extern "C" {
            #[link_name = "log"]
            fn pylon_log(level: i32, ptr: *const u8, len: usize);
        }
        // SAFETY: the host reads `len` bytes at `ptr` from this module's
        // memory and keeps no reference.
        unsafe { pylon_log(level as i32, message.as_ptr(), message.len()) }
    }
    #[cfg(not(target_family = "wasm"))]
    eprintln!("[shard {level:?}] {message}");
}

/// The codec the host chose for inputs and snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Json,
    MessagePack,
}

impl Codec {
    fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Codec::Json),
            1 => Some(Codec::MessagePack),
            _ => None,
        }
    }

    pub fn decode<T: DeserializeOwned>(self, bytes: &[u8]) -> Result<T, String> {
        match self {
            Codec::Json => serde_json::from_slice(bytes).map_err(|e| format!("json: {e}")),
            Codec::MessagePack => rmp_serde::from_slice(bytes).map_err(|e| format!("msgpack: {e}")),
        }
    }

    pub fn encode_into<T: Serialize>(self, value: &T, out: &mut Vec<u8>) -> Result<(), String> {
        out.clear();
        match self {
            Codec::Json => {
                serde_json::to_writer(&mut *out, value).map_err(|e| format!("json: {e}"))
            }
            Codec::MessagePack => {
                rmp_serde::encode::write_named(out, value).map_err(|e| format!("msgpack: {e}"))
            }
        }
    }
}

/// Generate the ABI exports for a [`Shard`] type. Use it once per module.
#[macro_export]
macro_rules! export_shard {
    ($ty:ty) => {
        #[cfg(target_family = "wasm")]
        const _: () = {
            static RUNTIME: $crate::__rt::Runtime<$ty> = $crate::__rt::Runtime::new();

            #[no_mangle]
            pub extern "C" fn pylon_shard_abi() -> i32 {
                $crate::ABI_VERSION
            }
            #[no_mangle]
            pub extern "C" fn pylon_scratch(len: i32) -> i32 {
                RUNTIME.scratch(len)
            }
            #[no_mangle]
            pub extern "C" fn pylon_output_ptr() -> i32 {
                RUNTIME.output_ptr()
            }
            #[no_mangle]
            pub extern "C" fn pylon_output_len() -> i32 {
                RUNTIME.output_len()
            }
            #[no_mangle]
            pub extern "C" fn pylon_init(codec: i32, ptr: i32, len: i32) -> i32 {
                RUNTIME.init(codec, ptr, len)
            }
            #[no_mangle]
            pub extern "C" fn pylon_apply_input(sp: i32, sl: i32, ip: i32, il: i32) -> i32 {
                RUNTIME.apply_input(sp, sl, ip, il)
            }
            #[no_mangle]
            pub extern "C" fn pylon_tick(dt_nanos: i64) {
                RUNTIME.tick(dt_nanos)
            }
            #[no_mangle]
            pub extern "C" fn pylon_snapshot() -> i32 {
                RUNTIME.snapshot()
            }
            #[no_mangle]
            pub extern "C" fn pylon_snapshot_for(sp: i32, sl: i32) -> i32 {
                RUNTIME.snapshot_for(sp, sl)
            }
            #[no_mangle]
            pub extern "C" fn pylon_is_finished() -> i32 {
                RUNTIME.is_finished()
            }
            #[no_mangle]
            pub extern "C" fn pylon_authorize_subscribe(sp: i32, sl: i32, ap: i32, al: i32) -> i32 {
                RUNTIME.authorize_subscribe(sp, sl, ap, al)
            }
            #[no_mangle]
            pub extern "C" fn pylon_authorize_input(
                sp: i32,
                sl: i32,
                ap: i32,
                al: i32,
                ip: i32,
                il: i32,
            ) -> i32 {
                RUNTIME.authorize_input(sp, sl, ap, al, ip, il)
            }
            #[no_mangle]
            pub extern "C" fn pylon_interest() -> i32 {
                RUNTIME.interest()
            }
            #[no_mangle]
            pub extern "C" fn pylon_entities() -> i32 {
                RUNTIME.entities()
            }
            #[no_mangle]
            pub extern "C" fn pylon_interest_area(sp: i32, sl: i32) -> i32 {
                RUNTIME.interest_area(sp, sl)
            }
            #[no_mangle]
            pub extern "C" fn pylon_filter_visible(sp: i32, sl: i32, ip: i32, il: i32) -> i32 {
                RUNTIME.filter_visible(sp, sl, ip, il)
            }
            #[no_mangle]
            pub extern "C" fn pylon_snapshot_visible(sp: i32, sl: i32, vp: i32, vl: i32) -> i32 {
                RUNTIME.snapshot_visible(sp, sl, vp, vl)
            }
            #[no_mangle]
            pub extern "C" fn pylon_replication() -> i32 {
                RUNTIME.replication()
            }
        };
    };
}

/// Support code for [`export_shard!`]. Not a stable API.
#[doc(hidden)]
pub mod __rt {
    use super::*;

    pub const OK: i32 = 0;
    pub const ERR: i32 = 1;
    pub const SAME_AS_BROADCAST: i32 = 2;

    struct State<T> {
        shard: Option<T>,
        codec: Codec,
        scratch: Vec<u8>,
        output: Vec<u8>,
        /// The store the host holds a full copy of (by `store_id`); only
        /// that store's changes can follow as a log.
        replication_synced: Option<u64>,
    }

    /// The module's global state. A `wasm32-unknown-unknown` module runs on
    /// one thread and the host never calls into it reentrantly, so one
    /// mutable borrow at a time holds.
    pub struct Runtime<T> {
        state: UnsafeCell<State<T>>,
    }

    // SAFETY: see the type docs. The module has one thread.
    unsafe impl<T> Sync for Runtime<T> {}

    impl<T: Shard> Default for Runtime<T> {
        fn default() -> Self {
            Self::new()
        }
    }

    impl<T: Shard> Runtime<T> {
        pub const fn new() -> Self {
            Self {
                state: UnsafeCell::new(State {
                    shard: None,
                    codec: Codec::Json,
                    scratch: Vec::new(),
                    output: Vec::new(),
                    replication_synced: None,
                }),
            }
        }

        #[allow(clippy::mut_from_ref)]
        fn state(&self) -> &mut State<T> {
            // SAFETY: single thread, no reentrancy; each export takes this
            // borrow once and drops it before returning.
            unsafe { &mut *self.state.get() }
        }

        pub fn scratch(&self, len: i32) -> i32 {
            let s = self.state();
            let len = len.max(0) as usize;
            if s.scratch.len() < len {
                s.scratch.resize(len, 0);
            }
            s.scratch.as_mut_ptr() as usize as i32
        }

        pub fn output_ptr(&self) -> i32 {
            self.state().output.as_ptr() as usize as i32
        }

        pub fn output_len(&self) -> i32 {
            self.state().output.len() as i32
        }

        pub fn init(&self, codec: i32, ptr: i32, len: i32) -> i32 {
            install_panic_hook();
            let s = self.state();
            let Some(codec) = Codec::from_i32(codec) else {
                return fail(&mut s.output, format!("unknown codec {codec}"));
            };
            s.codec = codec;
            #[derive(Deserialize)]
            struct Init<P> {
                shard: String,
                params: P,
            }
            // SAFETY: the host wrote `len` bytes at `ptr` inside the scratch buffer.
            let bytes = unsafe { arg(ptr, len) };
            let init: Init<T::Params> = match serde_json::from_slice(bytes) {
                Ok(v) => v,
                Err(e) => return fail(&mut s.output, format!("invalid params: {e}")),
            };
            match T::init(&init.shard, init.params) {
                Ok(shard) => {
                    s.shard = Some(shard);
                    OK
                }
                Err(e) => fail(&mut s.output, e),
            }
        }

        pub fn apply_input(&self, sp: i32, sl: i32, ip: i32, il: i32) -> i32 {
            let s = self.state();
            // SAFETY: host-written arguments in the scratch buffer.
            let (sid, input) = unsafe { (arg_str(sp, sl), arg(ip, il)) };
            let input: T::Input = match s.codec.decode(input) {
                Ok(v) => v,
                Err(e) => return fail(&mut s.output, format!("invalid input: {e}")),
            };
            match shard(&mut s.shard).apply_input(sid, input) {
                Ok(()) => OK,
                Err(e) => fail(&mut s.output, e),
            }
        }

        pub fn tick(&self, dt_nanos: i64) {
            let dt = Duration::from_nanos(dt_nanos.max(0) as u64);
            shard(&mut self.state().shard).tick(dt);
        }

        pub fn snapshot(&self) -> i32 {
            let s = self.state();
            let snap = shard(&mut s.shard).snapshot();
            encode(s.codec, &snap, &mut s.output)
        }

        pub fn snapshot_for(&self, sp: i32, sl: i32) -> i32 {
            let s = self.state();
            // SAFETY: host-written argument in the scratch buffer.
            let sid = unsafe { arg_str(sp, sl) };
            match shard(&mut s.shard).snapshot_for(sid) {
                Some(snap) => encode(s.codec, &snap, &mut s.output),
                None => SAME_AS_BROADCAST,
            }
        }

        pub fn is_finished(&self) -> i32 {
            shard(&mut self.state().shard).is_finished() as i32
        }

        pub fn authorize_subscribe(&self, sp: i32, sl: i32, ap: i32, al: i32) -> i32 {
            let s = self.state();
            // SAFETY: host-written arguments in the scratch buffer.
            let (sid, auth) = unsafe { (arg_str(sp, sl), arg(ap, al)) };
            let auth: Auth = match serde_json::from_slice(auth) {
                Ok(a) => a,
                Err(e) => return fail(&mut s.output, format!("invalid auth: {e}")),
            };
            match shard(&mut s.shard).authorize_subscribe(sid, &auth) {
                Ok(()) => OK,
                Err(e) => fail(&mut s.output, e),
            }
        }

        pub fn authorize_input(&self, sp: i32, sl: i32, ap: i32, al: i32, ip: i32, il: i32) -> i32 {
            let s = self.state();
            // SAFETY: host-written arguments in the scratch buffer.
            let (sid, auth, input) = unsafe { (arg_str(sp, sl), arg(ap, al), arg(ip, il)) };
            let auth: Auth = match serde_json::from_slice(auth) {
                Ok(a) => a,
                Err(e) => return fail(&mut s.output, format!("invalid auth: {e}")),
            };
            let input: T::Input = match s.codec.decode(input) {
                Ok(v) => v,
                Err(e) => return fail(&mut s.output, format!("invalid input: {e}")),
            };
            match shard(&mut s.shard).authorize_input(sid, &auth, &input) {
                Ok(()) => OK,
                Err(e) => fail(&mut s.output, e),
            }
        }
    }

    impl<T: Shard> Runtime<T> {
        pub fn interest(&self) -> i32 {
            let s = self.state();
            match shard(&mut s.shard).interest() {
                None => 0,
                Some(c) => {
                    s.output.clear();
                    s.output.extend_from_slice(&c.cell_size.to_le_bytes());
                    s.output.extend_from_slice(&c.margin.to_le_bytes());
                    s.output
                        .extend_from_slice(&(c.shared_snapshots as u32).to_le_bytes());
                    1
                }
            }
        }

        pub fn entities(&self) -> i32 {
            let s = self.state();
            let mut list = Vec::new();
            shard(&mut s.shard).entities(&mut list);
            s.output.clear();
            s.output.reserve(list.len() * 16);
            for e in list {
                s.output.extend_from_slice(&e.id.to_le_bytes());
                s.output.extend_from_slice(&e.x.to_le_bytes());
                s.output.extend_from_slice(&e.y.to_le_bytes());
            }
            OK
        }

        pub fn interest_area(&self, sp: i32, sl: i32) -> i32 {
            let s = self.state();
            // SAFETY: host-written argument in the scratch buffer.
            let sid = unsafe { arg_str(sp, sl) };
            match shard(&mut s.shard).interest_area(sid) {
                None => 0,
                Some(a) => {
                    s.output.clear();
                    for v in [a.x, a.y, a.radius] {
                        s.output.extend_from_slice(&v.to_le_bytes());
                    }
                    1
                }
            }
        }

        pub fn filter_visible(&self, sp: i32, sl: i32, ip: i32, il: i32) -> i32 {
            let s = self.state();
            // SAFETY: host-written arguments in the scratch buffer.
            let (sid, ids) = unsafe { (arg_str(sp, sl), arg(ip, il)) };
            let game = shard(&mut s.shard);
            s.output.clear();
            for chunk in ids.chunks_exact(8) {
                let id = u64::from_le_bytes(chunk.try_into().unwrap());
                if game.can_see(sid, id) {
                    s.output.extend_from_slice(chunk);
                }
            }
            OK
        }

        pub fn replication(&self) -> i32 {
            let s = self.state();
            let game = shard(&mut s.shard);
            let config = game.replication_config();
            let Some(store) = game.replicated() else {
                s.replication_synced = None;
                return 0;
            };
            let (full, log) = if s.replication_synced == Some(store.store_id()) {
                (false, store.take_changes())
            } else {
                // A store the host has not seen (the first, or one the game
                // swapped in): its entities predate recording, so send them
                // all once and record from here on.
                store.record_changes(true);
                (true, store.full_changes())
            };
            s.replication_synced = Some(store.store_id());
            s.output.clear();
            s.output.extend_from_slice(&config.precision.to_le_bytes());
            s.output
                .extend_from_slice(&config.max_bytes_per_tick.to_le_bytes());
            s.output.push(config.y_up as u8 | (full as u8) << 1);
            s.output.extend_from_slice(&log);
            1
        }

        pub fn snapshot_visible(&self, sp: i32, sl: i32, vp: i32, vl: i32) -> i32 {
            let s = self.state();
            // SAFETY: host-written arguments in the scratch buffer.
            let (sid, raw) = unsafe { (arg_str(sp, sl), arg(vp, vl)) };
            let Some(ids) = parse_view(raw) else {
                return fail(&mut s.output, "malformed view".into());
            };
            let (nv, ne) = (ids.0, ids.1);
            let all = &ids.3;
            let view = View {
                visible: &all[..nv],
                entered: &all[nv..nv + ne],
                left: &all[nv + ne..],
            };
            match shard(&mut s.shard).snapshot_visible(sid, &view) {
                Some(snap) => encode(s.codec, &snap, &mut s.output),
                None => SAME_AS_BROADCAST,
            }
        }
    }

    /// (visible count, entered count, left count, all ids).
    fn parse_view(raw: &[u8]) -> Option<(usize, usize, usize, Vec<u64>)> {
        let count = |i: usize| -> Option<usize> {
            Some(u32::from_le_bytes(raw.get(i..i + 4)?.try_into().ok()?) as usize)
        };
        let (nv, ne, nl) = (count(0)?, count(4)?, count(8)?);
        let body = raw.get(12..)?;
        if body.len() != (nv + ne + nl) * 8 {
            return None;
        }
        let ids = body
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        Some((nv, ne, nl, ids))
    }

    fn shard<T>(slot: &mut Option<T>) -> &mut T {
        match slot {
            Some(s) => s,
            // The host calls pylon_init first and drops the module when it
            // fails, so this is a host bug. Trap.
            None => panic!("pylon shard: called before pylon_init succeeded"),
        }
    }

    fn encode<S: Serialize>(codec: Codec, snap: &S, out: &mut Vec<u8>) -> i32 {
        match codec.encode_into(snap, out) {
            Ok(()) => OK,
            Err(e) => fail(out, format!("snapshot encode failed: {e}")),
        }
    }

    fn fail(out: &mut Vec<u8>, message: String) -> i32 {
        out.clear();
        out.extend_from_slice(message.as_bytes());
        ERR
    }

    unsafe fn arg<'a>(ptr: i32, len: i32) -> &'a [u8] {
        if len <= 0 {
            return &[];
        }
        std::slice::from_raw_parts(ptr as usize as *const u8, len as usize)
    }

    unsafe fn arg_str<'a>(ptr: i32, len: i32) -> &'a str {
        // The host sends subscriber ids as UTF-8 (they come from a URL).
        std::str::from_utf8(arg(ptr, len)).unwrap_or("")
    }

    fn install_panic_hook() {
        std::panic::set_hook(Box::new(|info| {
            log(Level::Error, &format!("panic: {info}"));
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_subscribe_rule_matches_the_host() {
        let user = |id: &str| Auth {
            user_id: Some(id.into()),
            ..Auth::default()
        };
        assert!(default_authorize_subscribe("u1", &user("u1")).is_ok());
        assert!(default_authorize_subscribe("u2", &user("u1")).is_err());
        assert!(default_authorize_subscribe("u1", &Auth::default()).is_err());
        let admin = Auth {
            is_admin: true,
            ..Auth::default()
        };
        assert!(default_authorize_subscribe("anyone", &admin).is_ok());
    }

    #[test]
    fn auth_decodes_the_host_shape() {
        let auth: Auth = serde_json::from_str(
            r#"{"user_id":"u1","is_admin":false,"roles":["gm"],"tenant_id":null,
                "ticket":{"shard":"s","sid":"c1","user_id":"u1","exp":9,"claims":{"realm":"n"}}}"#,
        )
        .unwrap();
        assert!(auth.has_role("gm"));
        assert_eq!(auth.claim("realm"), Some(&serde_json::json!("n")));
    }

    #[test]
    fn codecs_round_trip() {
        let mut out = Vec::new();
        for codec in [Codec::Json, Codec::MessagePack] {
            codec.encode_into(&("a", 2u8), &mut out).unwrap();
            let back: (String, u8) = codec.decode(&out).unwrap();
            assert_eq!(back, ("a".into(), 2));
        }
    }
}

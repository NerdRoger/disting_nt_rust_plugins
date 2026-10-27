//! Safe plugin callbacks with concrete engine, UI, and parameter modules.
#![no_std]
#![cfg_attr(not(test), forbid(unsafe_code))]
#![cfg_attr(test, deny(unsafe_code))]
#![deny(unsafe_op_in_unsafe_fn)]

use distingnt_api::*;
mod engine;
use engine::Channel;
mod controls;
mod drawing;
mod model;
use model::{ChannelCount, State};
mod plugin;
use plugin::WindowComparator;
mod parameter_changes;
mod parameters;
mod preset;
use parameters::{aggregate as agg, channel as cp, counted_gate as gate, global as gp};
#[cfg(test)]
// The simulated C host deliberately uses raw pointers throughout its fixtures.
#[allow(unsafe_code, unsafe_op_in_unsafe_fn)]
mod tests;
mod ui;

type Callback<'a, Mode> = distingnt_api::api::plugin::Context<'a, State, Channel, Mode>;

const CHANNEL_PARAMS: usize = cp::COUNT;
const COMMON_AGG: usize = gp::COUNT + cp::COUNT;

fn count_parameters(n: usize) -> usize {
    if n == 1 {
        gp::COUNT + cp::COUNT
    } else {
        COMMON_AGG + agg::COUNT + (n - 1) * (cp::COUNT + gate::COUNT)
    }
}
fn channel_offset(ch: usize) -> usize {
    if ch == 0 {
        gp::COUNT
    } else {
        COMMON_AGG + agg::COUNT + (ch - 1) * (cp::COUNT + gate::COUNT)
    }
}

distingnt_api::export_plugin!(WindowComparator);

#[cfg(test)]
type Framework = distingnt_api::api::plugin::Registration<WindowComparator, 1>;
#[cfg(test)]
type Algorithm = distingnt_sys::NtAlgorithm;

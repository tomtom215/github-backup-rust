// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! One module per screen.  Each module exposes a `render_*` free function that
//! borrows the relevant slice of `App` state and draws into a `Frame`.

pub mod configure;
pub mod dashboard;
pub mod results;
pub mod running;
pub mod verify;

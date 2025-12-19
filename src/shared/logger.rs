//! Re-export logger API from the external `nu_logger` crate to
//! preserve the `nu_analytics::shared::logger` path.

pub use nu_logger::{
	disable_debug,
	enable_debug,
	is_debug_enabled,
	set_level,
	set_level_from_str,
	Level,
};

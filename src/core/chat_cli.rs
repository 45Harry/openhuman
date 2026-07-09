//! Thin transport adapter for `openhuman chat`.

use anyhow::Result;

pub fn run_chat_command(args: &[String]) -> Result<()> {
    crate::openhuman::chat_cli::run_chat_command(args)
}

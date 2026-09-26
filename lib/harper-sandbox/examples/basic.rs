// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright 2026 coccinella-labs

use harper_sandbox::{Sandbox, SandboxConfig, SandboxRequest};

fn main() {
    println!("Harper Sandbox Test\n");

    let config = SandboxConfig::default();
    let sandbox = Sandbox::new(config);
    let request = SandboxRequest::new("echo", &["hello from sandbox example"]).unwrap();

    println!("Backend: {}", sandbox.backend_name());
    println!("Available: {}", sandbox.is_available());
    println!("Example request: {:?}", request);
    println!("\n✓ harper-sandbox crate is working!");

    println!("\nTo enable sandbox, set in config:");
    println!("  [exec_policy]");
    println!("  sandbox_profile = \"workspace\"");
}

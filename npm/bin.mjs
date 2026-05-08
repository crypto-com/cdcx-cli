#!/usr/bin/env node

import { execFileSync } from "child_process";
import { existsSync } from "fs";
import { findBinary, install, ROOT_DIR, LOCAL_BIN } from "./lib.mjs";

let bin = findBinary();
if (!bin) {
  try {
    install();
  } catch {
    // global install failed — fall through to local attempt
  }
  bin = findBinary();
  if (!bin) {
    try {
      install(ROOT_DIR);
    } catch {
      process.stderr.write("cdcx installation failed.\n");
      process.exit(1);
    }
    if (!existsSync(LOCAL_BIN)) {
      process.stderr.write("cdcx installation failed — binary not found.\n");
      process.exit(1);
    }
    bin = LOCAL_BIN;
  }
}

try {
  execFileSync(bin, process.argv.slice(2), { stdio: "inherit" });
} catch (e) {
  process.exit(e.status ?? 1);
}

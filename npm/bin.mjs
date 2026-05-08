#!/usr/bin/env node

import { execFileSync, execSync } from "child_process";
import { existsSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));

function findBinary() {
  const local = join(__dirname, "..", "cdcx");
  if (existsSync(local)) return local;
  try {
    return execSync("command -v cdcx", { encoding: "utf8" }).trim();
  } catch {
    return null;
  }
}

function install(dir) {
  const script = join(__dirname, "..", "install.sh");
  if (!existsSync(script)) {
    process.stderr.write(
      "cdcx not found. Install it: curl -sSfL https://raw.githubusercontent.com/crypto-com/cdcx-cli/main/install.sh | sh\n"
    );
    process.exit(1);
  }
  const env = dir ? { ...process.env, INSTALL_DIR: dir } : process.env;
  execFileSync("sh", [script], { stdio: "inherit", env });
}

let bin = findBinary();
if (!bin) {
  install();
  bin = findBinary();
  if (!bin) {
    const localDir = join(__dirname, "..");
    install(localDir);
    const localBin = join(localDir, "cdcx");
    if (!existsSync(localBin)) {
      process.stderr.write("cdcx installation failed — binary not found.\n");
      process.exit(1);
    }
    bin = localBin;
  }
}

try {
  execFileSync(bin, process.argv.slice(2), { stdio: "inherit" });
} catch (e) {
  process.exit(e.status ?? 1);
}

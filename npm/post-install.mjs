import { readFileSync, existsSync } from "fs";
import { join } from "path";
import { homedir } from "os";
import {
  findBinary,
  install,
  installedVersion,
  ROOT_DIR,
} from "./lib.mjs";

function isUpdateDisabled() {
  try {
    const configPath = join(homedir(), ".config", "cdcx", "config.toml");
    if (!existsSync(configPath)) return false;
    const content = readFileSync(configPath, "utf8");
    return /^\s*disable_update_check\s*=\s*"?true"?\s*$/m.test(content);
  } catch {
    return false;
  }
}

const bin = findBinary();

if (!bin) {
  // No binary at all — must install regardless of update preference
  try {
    install();
  } catch {
    try {
      install(ROOT_DIR);
    } catch {
      process.stderr.write("cdcx installation failed.\n");
      process.exit(1);
    }
  }
} else if (!isUpdateDisabled()) {
  const expected = JSON.parse(
    readFileSync(join(ROOT_DIR, "package.json"), "utf8")
  ).version;
  const current = installedVersion(bin);

  if (current !== expected) {
    try {
      install();
    } catch {
      // global failed — try local
    }
    const updated = findBinary();
    if (!updated || installedVersion(updated) !== expected) {
      try {
        install(ROOT_DIR);
      } catch {
        process.stderr.write("cdcx installation failed.\n");
        process.exit(1);
      }
    }
  }
}

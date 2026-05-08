import { readFileSync } from "fs";
import { join } from "path";
import {
  findBinary,
  install,
  installedVersion,
  ROOT_DIR,
} from "./lib.mjs";

const expected = JSON.parse(
  readFileSync(join(ROOT_DIR, "package.json"), "utf8")
).version;

const bin = findBinary();
const current = bin ? installedVersion(bin) : null;

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

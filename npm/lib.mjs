import { execFileSync, execSync } from "child_process";
import { existsSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
export const BINARY = "cdcx";
export const ROOT_DIR = join(__dirname, "..");
export const LOCAL_BIN = join(ROOT_DIR, BINARY);
const INSTALL_SCRIPT = join(ROOT_DIR, "install.sh");

export function findBinary() {
  if (existsSync(LOCAL_BIN)) {
    try {
      execFileSync(LOCAL_BIN, ["--version"], { stdio: "pipe" });
      return LOCAL_BIN;
    } catch {
      // exists but not functional — fall through
    }
  }
  try {
    const bin = execSync(`command -v ${BINARY}`, { encoding: "utf8" }).trim();
    execFileSync(bin, ["--version"], { stdio: "pipe" });
    return bin;
  } catch {
    return null;
  }
}

export function installedVersion(bin) {
  try {
    const out = execFileSync(bin, ["--version"], { encoding: "utf8" }).trim();
    const match = out.match(/(\d+\.\d+\.\d+)/);
    return match ? match[1] : null;
  } catch {
    return null;
  }
}

export function install(dir) {
  if (!existsSync(INSTALL_SCRIPT)) {
    process.stderr.write(
      `${BINARY} not found. Install it: curl -sSfL https://raw.githubusercontent.com/crypto-com/cdcx-cli/main/install.sh | sh\n`
    );
    process.exit(1);
  }
  const env = dir ? { ...process.env, INSTALL_DIR: dir } : process.env;
  execFileSync("sh", [INSTALL_SCRIPT], {
    stdio: ["pipe", process.stderr, process.stderr],
    env,
  });
}

import { execFileSync, execSync } from "child_process";
import { existsSync, readFileSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const expected = JSON.parse(
  readFileSync(join(__dirname, "..", "package.json"), "utf8")
).version;

function installedVersion() {
  try {
    const out = execFileSync(join(__dirname, "bin.mjs"), ["--version"], {
      encoding: "utf8",
    }).trim();
    return out.replace(/^cdcx\s+/, "");
  } catch {
    return null;
  }
}

function install(dir) {
  const script = join(__dirname, "..", "install.sh");
  const env = dir ? { ...process.env, INSTALL_DIR: dir } : process.env;
  execFileSync(script, { stdio: "inherit", env });
}

if (installedVersion() !== expected) {
  install();
  try {
    execSync("command -v cdcx", { encoding: "utf8" });
  } catch {
    const localDir = join(__dirname, "..");
    install(localDir);
    if (!existsSync(join(localDir, "cdcx"))) {
      process.stderr.write("cdcx installation failed — binary not found.\n");
      process.exit(1);
    }
  }
}

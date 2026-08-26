#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";

const platform = `${process.platform}-${process.arch}`;
const packageName = `@microck/pangram-cli-${platform}`;
const require = createRequire(import.meta.url);
const manifest = require("../package.json");
if (!Object.hasOwn(manifest.optionalDependencies, packageName)) {
  console.error(`pangram: unsupported platform ${platform}`);
  process.exit(1);
}

let executable;
try {
  executable = require.resolve(
    `${packageName}/bin/${process.platform === "win32" ? "pangram.exe" : "pangram"}`,
  );
} catch {
  console.error(`pangram: required platform package ${packageName} is not installed`);
  process.exit(1);
}

const child = spawnSync(executable, process.argv.slice(2), { stdio: "inherit" });
if (child.error) {
  console.error(`pangram: could not start the platform executable: ${child.error.message}`);
  process.exit(1);
}
if (child.signal) {
  process.kill(process.pid, child.signal);
}
process.exit(child.status ?? 1);

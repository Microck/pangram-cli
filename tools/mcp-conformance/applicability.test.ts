import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { createRequire } from "node:module";

import { describe, expect, it } from "vitest";

const require = createRequire(import.meta.url);
const packageJsonPath = require.resolve(
  "@modelcontextprotocol/conformance/package.json",
);
const packageRoot = dirname(packageJsonPath);
const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8")) as {
  bin?: Record<string, string>;
  version?: string;
};

function conformanceCliPath(): string {
  const executable = packageJson.bin?.conformance;
  if (executable === undefined) {
    throw new Error("The conformance package does not expose its CLI.");
  }

  return join(packageRoot, executable);
}

describe("official MCP conformance applicability", () => {
  it("pins the audited official suite release", () => {
    expect(packageJson.version).toBe("0.2.0-alpha.11");
    expect(packageJson.bin).toEqual({ conformance: "dist/index.js" });
  });

  it("offers only a URL-backed server runner", () => {
    // `server --help` exercises the published CLI surface without connecting to,
    // launching, or substituting any Pangram server.
    const help = execFileSync(
      process.execPath,
      [conformanceCliPath(), "server", "--help"],
      { encoding: "utf8" },
    );

    expect(help).toContain("--url <url>");
    expect(help).not.toMatch(/stdio/i);
  });

  it("requires the frozen diagnostic server profile", () => {
    const requirements = readFileSync(
      join(packageRoot, "requirements", "2026-07-28.yaml"),
      "utf8",
    );
    const serverProfile = requirements.slice(
      requirements.indexOf("server:\n"),
      requirements.indexOf("\nclient:\n"),
    );

    expect(requirements).toContain("This file is the canonical answer");
    expect(requirements).toContain("It is FROZEN:");

    for (const scenario of [
      "completion-complete",
      "dns-rebinding-protection",
      "tools-list",
      "tools-call-simple-text",
      "tools-call-image",
      "tools-call-audio",
      "tools-call-embedded-resource",
      "tools-call-mixed-content",
      "tools-call-error",
      "tools-call-with-progress",
      "resources-list",
      "resources-read-text",
      "resources-read-binary",
      "resources-templates-read",
      "prompts-list",
      "prompts-get-simple",
      "server-sse-multiple-streams",
      "input-required-result-basic-elicitation",
      "input-required-result-basic-sampling",
      "input-required-result-basic-list-roots",
      "input-required-result-request-state",
    ]) {
      expect(serverProfile).toContain(`  - ${scenario}\n`);
    }
  });
});

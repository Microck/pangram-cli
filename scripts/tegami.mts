#!/usr/bin/env node
import { tegami } from "tegami";
import { runCli } from "tegami/cli";
import { cargo } from "tegami/plugins/cargo";
import { github } from "tegami/plugins/github";
import { platformPackagesPlugin } from "./platform-packages-plugin.mts";

const paper = tegami({
  plugins: [
    cargo(),
    github({
      repo: "Microck/pangram-cli",
      createTags: false,
      release: false,
      versionPr: { base: "main" },
    }),
    platformPackagesPlugin(),
  ],
  npm: {
    client: "npm",
    updateLockFile: true,
    trustedPublish: {
      provider: "github",
      workflow: "publish.yml",
    },
  },
});

await runCli(paper);

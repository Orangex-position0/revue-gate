import { readFileSync } from "node:fs";

const messagePath = process.argv[2];

if (!messagePath) {
  console.error("Missing commit message file path.");
  process.exit(1);
}

const message = readFileSync(messagePath, "utf8")
  .split(/\r?\n/)
  .find((line) => line.trim() && !line.startsWith("#"))
  ?.trim();

if (!message) {
  console.error("Commit message is empty.");
  process.exit(1);
}

const allowedTypes = [
  "build",
  "chore",
  "ci",
  "docs",
  "feat",
  "fix",
  "perf",
  "refactor",
  "revert",
  "style",
  "test",
];

const typePattern = allowedTypes.join("|");
const conventionalCommit = new RegExp(
  `^(${typePattern})(\\([a-z0-9][a-z0-9._-]*\\))?!?: .{1,72}$`,
);

if (!conventionalCommit.test(message)) {
  console.error("Commit message must follow Conventional Commits:");
  console.error("  <type>(optional-scope): <subject>");
  console.error(`Allowed types: ${allowedTypes.join(", ")}`);
  console.error(`Received: ${message}`);
  process.exit(1);
}

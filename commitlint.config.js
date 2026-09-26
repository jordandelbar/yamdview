module.exports = {
  extends: ["@commitlint/config-conventional"],
  rules: {
      "type-enum": [
        2,
        "always",
      [
        "feat", "fix", "docs", "style", "refactor",
        "perf", "test", "build", "ci", "chore",
        "revert", "infra"
      ],
    ],
    "subject-case": [2, "always", "sentence-case"],
  },
  // Dependabot titles start with a lowercase "bump".
  ignores: [(message) => message.startsWith("chore(deps): bump ")],
  plugins: [],
};

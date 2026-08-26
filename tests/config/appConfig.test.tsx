import { describe, expect, it } from "vitest";
import {
  ADDITIVE_APP_IDS,
  APP_IDS,
  DEFAULT_VISIBLE_APPS,
  MCP_APP_IDS,
  PROXY_APP_IDS,
  SKILLS_APP_IDS,
  isAdditiveAppId,
  normalizeVisibleApps,
} from "@/config/appConfig";

describe("appConfig provider lifecycle", () => {
  it.each(["opencode"])("classifies %s as additive", (appId) => {
    expect(isAdditiveAppId(appId)).toBe(true);
  });

  it.each([
    "claude",
    "claude-desktop",
    "codex",
    "workbuddy",
    "gemini",
    "grokbuild",
  ])(
    "does not classify %s as additive",
    (appId) => {
      expect(isAdditiveAppId(appId)).toBe(false);
    },
  );

  it("exposes Codex, OpenCode, and WorkBuddy at product level", () => {
    expect(APP_IDS).toEqual(["codex", "opencode", "workbuddy"]);
    expect(SKILLS_APP_IDS).toEqual(["codex", "opencode"]);
    expect(MCP_APP_IDS).toEqual(["codex", "opencode"]);
    expect(PROXY_APP_IDS).toEqual(["codex", "workbuddy"]);
    expect(ADDITIVE_APP_IDS).toEqual(["opencode"]);
  });

  it("ignores legacy visibility flags for removed apps", () => {
    expect(
      normalizeVisibleApps({
        ...DEFAULT_VISIBLE_APPS,
        claude: true,
        codex: false,
        opencode: false,
      }),
    ).toEqual({
      ...DEFAULT_VISIBLE_APPS,
      codex: false,
      opencode: false,
    });
  });
});

import { describe, expect, it } from "vitest";
import { feedKindLabel, formatDaemonEndpoint } from "../../../desktop/cortex-control-center/src/app/utils/format.js";

describe("formatDaemonEndpoint", () => {
  it("keeps host and port from a valid cortexBase", () => {
    expect(formatDaemonEndpoint("http://127.0.0.1:7437")).toBe("127.0.0.1:7437");
  });

  it("falls back to the default local endpoint when cortexBase is not a URL", () => {
    expect(formatDaemonEndpoint("not a url")).toBe("127.0.0.1:7437");
  });
});

describe("feedKindLabel", () => {
  it("maps known feed kinds instead of throwing on the label table", () => {
    expect(feedKindLabel("prompt")).toBe("Prompt");
    expect(feedKindLabel("task_complete")).toBe("Task Complete");
  });

  it("returns Unknown for an empty kind", () => {
    expect(feedKindLabel("")).toBe("Unknown");
  });
});

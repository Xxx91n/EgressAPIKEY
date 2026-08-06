import { describe, it, expect } from "vitest";
import { policyToI18nKey } from "./policy";

describe("policyToI18nKey", () => {
  it("maps known policies to i18n keys", () => {
    expect(policyToI18nKey("BALANCED")).toBe("policy.balanced");
    expect(policyToI18nKey("PREFER_LOW_LATENCY")).toBe("policy.lowLatency");
    expect(policyToI18nKey("PREFER_IDLE_IP")).toBe("policy.idleIp");
  });

  it("passes unknown values through as-is", () => {
    expect(policyToI18nKey("SOMETHING_NEW")).toBe("SOMETHING_NEW");
    expect(policyToI18nKey("")).toBe("");
  });
});

import { describe, expect, it } from "vitest";
import { matchClass, matchWords } from "./wallet-words";

describe("the three-way match's words", () => {
  it("calls a row matched only when PayPal was read back", () => {
    expect(matchWords({ state: "matched", paypalStatus: "COMPLETED" })).toBe("Matched");
    expect(matchClass({ state: "matched", paypalStatus: "COMPLETED" })).toBe("ax-ok");
    expect(matchWords({ state: "matched", paypalStatus: null })).toBe("In the books · PayPal not read back");
    expect(matchClass({ state: "matched", paypalStatus: null })).toBe("ax-wait");
  });

  it("names every difference", () => {
    expect(matchWords({ state: "not_in_books", paypalStatus: "COMPLETED" })).toBe("Not in the books");
    expect(matchWords({ state: "pay_pal_differs", paypalStatus: "REFUNDED" })).toBe("PayPal differs");
    expect(matchWords({ state: "books_differ", paypalStatus: "COMPLETED" })).toBe("Books differ");
    expect(matchClass({ state: "books_differ", paypalStatus: "COMPLETED" })).toBe("ax-risk");
  });
});

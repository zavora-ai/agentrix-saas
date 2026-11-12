// The three-way match's words, apart from the grid so they can be tested without a DOM.

export interface MatchLike {
  state: string;
  /** What PayPal said when read back; null when PayPal wasn't read back. */
  paypalStatus: string | null;
}

const STATE: Record<string, string> = {
  matched: "Matched",
  not_in_books: "Not in the books",
  pay_pal_differs: "PayPal differs",
  paypal_differs: "PayPal differs",
  books_differ: "Books differ",
};

/** A match row's words: a row PayPal wasn't read back for is never called matched three ways. */
export function matchWords(m: MatchLike): string {
  if (m.state === "matched" && m.paypalStatus === null) return "In the books · PayPal not read back";
  return STATE[m.state] ?? m.state;
}

/** Its colour: green only when all three agree. */
export function matchClass(m: MatchLike): "ax-ok" | "ax-risk" | "ax-wait" {
  if (m.state !== "matched") return "ax-risk";
  return m.paypalStatus === null ? "ax-wait" : "ax-ok";
}

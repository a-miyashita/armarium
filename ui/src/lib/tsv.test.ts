import { expect, it } from "vitest";
import { toTsv } from "./tsv";

it("quotes fields like Excel", () => {
  expect(toTsv([["a", "b\tc"], ['say "hi"', "x\ny"]])).toBe('a\t"b\tc"\r\n"say ""hi"""\t"x\ny"');
});

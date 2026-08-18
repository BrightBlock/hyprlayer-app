// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { PromptCard } from "./PromptCard";

afterEach(cleanup);

const BODY = "Refactor this module.\nExtract the parser.\nAdd tests.";

/** Match the body verbatim, the default matcher collapses the newlines that
 *  make a prompt body worth collapsing in the first place. */
const revealedBody = () => screen.queryByText(BODY, { normalizer: (text) => text });

describe("PromptCard", () => {
  it("shows the prompt's name and keeps the body out of the transcript", () => {
    render(<PromptCard name="Refactor for testability" body={BODY} />);

    expect(screen.getByText("Refactor for testability")).toBeTruthy();
    expect(revealedBody()).toBeNull();
  });

  it("expands in place to the exact body the agent received", () => {
    render(<PromptCard name="Refactor for testability" body={BODY} />);
    const toggle = screen.getByRole("button");
    expect(toggle.getAttribute("aria-expanded")).toBe("false");

    fireEvent.click(toggle);

    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(revealedBody()?.textContent).toBe(BODY);
  });

  it("collapses again on a second click", () => {
    render(<PromptCard name="Refactor for testability" body={BODY} />);
    const toggle = screen.getByRole("button");

    fireEvent.click(toggle);
    fireEvent.click(toggle);

    expect(revealedBody()).toBeNull();
  });

  it("counts the body's lines so a long prompt reads as long while collapsed", () => {
    render(<PromptCard name="Refactor" body={BODY} />);
    expect(screen.getByText("3 lines")).toBeTruthy();
  });

  it("singularizes the line count", () => {
    render(<PromptCard name="Refactor" body="one liner" />);
    expect(screen.getByText("1 line")).toBeTruthy();
  });
});

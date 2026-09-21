import { useState } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { Select } from "./Select";

afterEach(cleanup);

const options = [
  { value: "a", label: "Alpha" },
  { value: "b", label: "Beta" },
  { value: "c", label: "Charlie", disabled: true }
];

function Harness({ onChange }: { onChange: (value: string) => void }) {
  const [value, setValue] = useState("a");
  return <Select value={value} options={options} onChange={(next) => { setValue(next); onChange(next); }} aria-label="Letter" />;
}

describe("Select", () => {
  it("opens a menu and selects with the mouse", async () => {
    const values: string[] = [];
    render(<Harness onChange={(value) => values.push(value)} />);
    fireEvent.click(screen.getByRole("button", { name: "Letter" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Beta" }));
    expect(values).toEqual(["b"]);
    expect(screen.getByRole("button", { name: "Letter" })).toHaveTextContent("Beta");
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("supports arrow-key navigation and skips disabled items", async () => {
    const values: string[] = [];
    render(<Harness onChange={(value) => values.push(value)} />);
    fireEvent.click(screen.getByRole("button", { name: "Letter" }));
    const menu = await screen.findByRole("menu");
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    fireEvent.keyDown(menu, { key: "ArrowDown" }); // skips disabled "Charlie", wraps to "Alpha"
    fireEvent.keyDown(menu, { key: "Enter" });
    expect(values).toEqual(["a"]);
  });

  it("closes on Escape", async () => {
    render(<Harness onChange={() => undefined} />);
    fireEvent.click(screen.getByRole("button", { name: "Letter" }));
    await screen.findByRole("menu");
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
  });
});

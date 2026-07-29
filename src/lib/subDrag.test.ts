import { describe, it, expect } from "vitest";
import { createDragState, beginDrag, enterTarget, finishDrag, cancelDrag } from "../lib/subDrag";

describe("subDrag", () => {
  it("no-op when not dragging", () => {
    const ds = createDragState();
    const out = finishDrag(ds, [{ a: 1 }, { a: 2 }]);
    expect(out).toEqual([{ a: 1 }, { a: 2 }]);
    expect(ds.isDragging).toBe(false);
  });

  it("moves an item forward", () => {
    const ds = createDragState();
    beginDrag(ds, 0);
    enterTarget(ds, 2);
    const out = finishDrag(ds, [0, 1, 2, 3]);
    expect(out).toEqual([1, 2, 0, 3]);
  });

  it("moves an item backward", () => {
    const ds = createDragState();
    beginDrag(ds, 3);
    enterTarget(ds, 1);
    const out = finishDrag(ds, ["a", "b", "c", "d"]);
    expect(out).toEqual(["a", "d", "b", "c"]);
  });

  it("ignores right-button drags", () => {
    const ds = createDragState();
    beginDrag(ds, 0, { button: 2 });
    expect(ds.isDragging).toBe(false);
    const out = finishDrag(ds, [1, 2, 3]);
    expect(out).toEqual([1, 2, 3]);
  });

  it("cancel resets state", () => {
    const ds = createDragState();
    beginDrag(ds, 1);
    cancelDrag(ds);
    expect(ds.dragIndex).toBe(null);
    expect(ds.isDragging).toBe(false);
  });
});

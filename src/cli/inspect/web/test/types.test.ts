import { describe, expect, it } from "vitest";
import {
  DEFAULT_LENS,
  DEFAULT_OPEN_FILES,
  LARGE_FILE_ROWS,
  LENSES,
  OVERLAY_SELECTORS,
  QUERY_FIELDS,
  SUPERSEDABLE_FACT_TYPES,
  TYPE_MAP,
  TYPES,
  typeColor,
  typeLabel,
} from "../src/types";

describe("TYPES", () => {
  it("lists the event types in their canonical order", () => {
    expect(TYPES.map((type) => type.id)).toEqual([
      "review_initialized",
      "work_object_proposed",
      "review_observation_recorded",
      "review_assessment_recorded",
      "input_request_opened",
      "input_request_responded",
      "review_note_imported",
      "validation_check_recorded",
    ]);
  });

  it("gives every type a label and a CSS palette colour", () => {
    for (const type of TYPES) {
      expect(type.label).toBeTruthy();
      expect(type.color).toMatch(/^var\(--evt-/);
    }
  });

  it("carries the current capture/validation types and not the retired lineage ones", () => {
    const ids = TYPES.map((type) => type.id);
    expect(ids).toContain("work_object_proposed");
    expect(ids).toContain("validation_check_recorded");
    expect(ids).not.toContain("review_unit_lineage");
    expect(ids).not.toContain("review_unit_captured");
  });
});

describe("TYPE_MAP", () => {
  it("indexes each type by id", () => {
    expect(TYPE_MAP.work_object_proposed?.label).toBe("capture");
    expect(TYPE_MAP.validation_check_recorded?.label).toBe("validation");
    expect(TYPE_MAP.unknown_id).toBeUndefined();
  });
});

describe("typeColor / typeLabel", () => {
  it("resolves the palette colour for a known id", () => {
    expect(typeColor("review_initialized")).toBe("var(--evt-init)");
    expect(typeColor("validation_check_recorded")).toBe(
      "var(--evt-validation)",
    );
  });

  it("falls back to the note colour for an unknown id", () => {
    expect(typeColor("not_a_real_type")).toBe("var(--evt-note)");
  });

  it("resolves the label for a known id and falls back to the id otherwise", () => {
    expect(typeLabel("work_object_proposed")).toBe("capture");
    expect(typeLabel("not_a_real_type")).toBe("not_a_real_type");
  });
});

describe("shared constants", () => {
  it("declares the lenses and default", () => {
    expect(LENSES).toEqual(["timeline", "list", "attention"]);
    expect(DEFAULT_LENS).toBe("timeline");
  });

  it("declares the query grammar fields including attention", () => {
    expect(QUERY_FIELDS).toEqual([
      "type",
      "track",
      "revision",
      "snapshot",
      "status",
      "attention",
    ]);
  });

  it("declares the diff sizing thresholds", () => {
    expect(DEFAULT_OPEN_FILES).toBe(10);
    expect(LARGE_FILE_ROWS).toBe(500);
  });

  it("maps overlay names to their root selectors", () => {
    expect(OVERLAY_SELECTORS).toEqual({
      palette: "#cmd-palette",
      help: "#key-help",
    });
  });

  it("treats the four review fact types as supersedable", () => {
    expect(SUPERSEDABLE_FACT_TYPES.has("review_observation_recorded")).toBe(
      true,
    );
    expect(SUPERSEDABLE_FACT_TYPES.has("review_assessment_recorded")).toBe(
      true,
    );
    expect(SUPERSEDABLE_FACT_TYPES.has("input_request_opened")).toBe(true);
    expect(SUPERSEDABLE_FACT_TYPES.has("validation_check_recorded")).toBe(true);
    expect(SUPERSEDABLE_FACT_TYPES.size).toBe(4);
  });
});

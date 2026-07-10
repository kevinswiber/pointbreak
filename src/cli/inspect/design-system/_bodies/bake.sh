#!/usr/bin/env bash
# Bake self-contained design-system preview cards: marker + inlined styles.css + body.
# Re-run after editing styles.css or any *.body.html fragment.
set -euo pipefail
DS="$(cd "$(dirname "$0")/.." && pwd)"
TOKENS="$DS/../assets/tokens.css"
STYLES="$DS/styles.css"

# Verify immutable brand inputs and the live palette before writing generated
# gallery files. Both checks are repository-local and network-free.
node "$DS/brand-check.mjs"
node "$DS/contrast-check.mjs"
node "$DS/contrast-check.mjs" --variant variants/instrument-neutral.css

# Publish a token file for the synced design-system project: the shared product
# tokens plus the self-hosted JetBrains Mono @font-face block (fonts.css). The
# project's styles.css @imports this, so the compiler resolves both the tokens
# and the font faces. The baked cards still inline only the product tokens
# ($TOKENS, system --mono stack, no webfont), so the inspector and the cards stay
# zero-webfont; this concatenated copy is solely the project's bindable layer.
# Gitignored — regenerated here, never committed.
cat "$TOKENS" "$DS/fonts.css" > "$DS/tokens.css"
echo "published tokens.css (+ @font-face)"

bake() {
  local body="$1" out="$2" group="$3" title="$4" with_fonts="${5:-}" theme="${6:-}" name="${7:-}" subtitle="${8:-}"
  local visual_variant="${VISUAL_VARIANT:-}" variant_css="${VARIANT_CSS:-}"
  # The light cards are identical snapshots with data-theme="light" on <html>; the
  # tokens.css [data-theme="light"] aliases do the re-theming, no JS toggle needed.
  local html_tag='<html lang="en">'
  if [ -n "$theme" ]; then html_tag="<html lang=\"en\" data-theme=\"$theme\">"; fi
  # @dsCard marker: always group; paired (dark + light twin) cards add name/subtitle so
  # the Design pane shows each theme beside its counterpart under one distinct label.
  local marker="<!-- @dsCard group=\"$group\""
  if [ -n "$name" ]; then marker="$marker name=\"$name\""; fi
  if [ -n "$subtitle" ]; then marker="$marker subtitle=\"$subtitle\""; fi
  marker="$marker -->"
  mkdir -p "$(dirname "$DS/$out")"
  {
    printf '%s\n' "$marker"
    printf '<!doctype html>\n%s\n  <head>\n    <meta charset="utf-8" />\n' "$html_tag"
    printf '    <title>%s</title>\n    <style>\n' "$title"
    cat "$TOKENS"
    cat "$STYLES"
    # Temporary visual candidates compose after the live component layer and
    # are emitted only into gitignored gallery cards.
    if [ -n "$variant_css" ]; then cat "$variant_css"; fi
    # Cards that demo the self-hosted font opt in (5th arg): inline the @font-face
    # faces, path-rewritten one dir up since cards live in a subdirectory.
    if [ -n "$with_fonts" ]; then sed 's#url("fonts/#url("../fonts/#g' "$DS/fonts.css"; fi
    # Theme scope wrapper: the Design gallery re-hosts card markup and drops
    # <html> attributes, so the data-theme marker must ride an inner element
    # (the <html> attribute is still set above for standalone rendering).
    if [ -n "$visual_variant" ]; then
      printf '    </style>\n  </head>\n  <body>\n    <div class="ds-card" data-theme="%s" data-visual-variant="%s">\n' "${theme:-dark}" "$visual_variant"
    else
      printf '    </style>\n  </head>\n  <body>\n    <div class="ds-card" data-theme="%s">\n' "${theme:-dark}"
    fi
    cat "$DS/_bodies/$body"
    printf '    </div>\n  </body>\n</html>\n'
  } > "$DS/$out"
  echo "baked $out"
}

bake_variant() {
  local variant="$1"
  shift
  VISUAL_VARIANT="$variant" VARIANT_CSS="$DS/variants/$variant.css" bake "$@"
}

bake_comparison() {
  local body="$1" baseline_out="$2" candidate_out="$3" scenario="$4" theme="$5"
  local theme_label
  case "$theme" in
    dark) theme_label="Dark theme" ;;
    light) theme_label="Light theme" ;;
    *) echo "unsupported comparison theme: $theme" >&2; return 1 ;;
  esac
  bake "$body" "$baseline_out" "Review comparison" "$scenario — baseline — $theme" "" "$theme" "$scenario — baseline — $theme" "$theme_label · current Review"
  bake_variant instrument-neutral "$body" "$candidate_out" "Review comparison" "$scenario — instrument-neutral — $theme" "" "$theme" "$scenario — candidate — $theme" "$theme_label · unshipped candidate"
}

# Dark cards. Every card has a light twin, so each dark card carries an explicit
# name and each light card carries a matching "— light" name plus subtitle.
bake foundations.body.html         foundations/foundations.html Foundations "Foundations — tokens"                with-fonts "" "Foundations — dark" "Dark theme"
bake navigation-topbar.body.html   navigation/topbar.html      Navigation "Navigation — top bar, tabs, stats"    "" "" "Navigation — dark" "Dark theme"
bake inputs-controls.body.html     inputs/controls.html        Inputs     "Inputs — toolbar, buttons, toggles"    "" "" "Inputs — dark" "Dark theme"
bake data-timeline.body.html       data/timeline.html          Data       "Data — split shell (timeline · divider · detail)" "" "" "Timeline — dark" "Dark theme"
bake data-cards.body.html          data/cards.html             Data       "Data — unit & revision-thread cards"  "" "" "Revision thread — dark" "Dark theme"
bake data-review-facts.body.html   data/review-facts.html      Data       "Data — verdict, facts, endorsements"  "" "" "Review facts — dark" "Dark theme"
bake data-diff.body.html           data/diff.html              Data       "Data — annotated diff"                "" "" "Annotated diff — dark" "Dark theme"
bake data-attention.body.html      data/attention.html         Data       "Data — Attention lens"                "" "" "Attention — dark" "Dark theme"
bake feedback-diagnostics.body.html feedback/diagnostics.html  Feedback   "Feedback — diagnostics & errors"      "" "" "Feedback — dark" "Dark theme"

# Temporary, unserved candidate proof. Task 3.2 owns the full comparison
# matrix; this pair only proves that the isolated override composes and bakes.
bake_variant instrument-neutral foundations.body.html foundations/foundations-instrument-neutral.html Foundations "Foundations — instrument-neutral candidate" with-fonts "" "Foundations — instrument-neutral dark" "Dark theme · unshipped candidate"

# Light-theme variants — paired beside their dark twin in the SAME group (not a
# separate one), each carrying a "— light" name + "Light theme" subtitle so the pair
# is unambiguous even where the pane shows only the name. The AA-tuned tokens.css
# [data-theme="light"] aliases re-theme everything; no JS toggle. The foundations light
# card keeps the @font-face opt-in (5th arg) for its mono ramp; the rest stay zero-webfont.
bake foundations.body.html         foundations/foundations-light.html Foundations "Foundations — light theme"     with-fonts light "Foundations — light"     "Light theme"
bake navigation-topbar.body.html   navigation/topbar-light.html       Navigation  "Navigation — light theme"     "" light "Navigation — light"      "Light theme"
bake inputs-controls.body.html     inputs/controls-light.html         Inputs      "Inputs — light theme"         "" light "Inputs — light"          "Light theme"
bake data-timeline.body.html       data/timeline-light.html           Data        "Data — split shell, light theme" "" light "Timeline — light"        "Light theme"
bake data-cards.body.html          data/cards-light.html              Data        "Data — revision thread, light" "" light "Revision thread — light" "Light theme"
bake data-review-facts.body.html   data/review-facts-light.html       Data        "Data — review facts, light"    "" light "Review facts — light"    "Light theme"
bake data-diff.body.html           data/diff-light.html               Data        "Data — annotated diff, light"  "" light "Annotated diff — light"  "Light theme"
bake data-attention.body.html      data/attention-light.html          Data        "Data — Attention lens, light"  "" light "Attention — light"       "Light theme"
bake feedback-diagnostics.body.html feedback/diagnostics-light.html   Feedback    "Feedback — light theme"       "" light "Feedback — light"        "Light theme"
bake_variant instrument-neutral foundations.body.html foundations/foundations-instrument-neutral-light.html Foundations "Foundations — instrument-neutral candidate, light" with-fonts light "Foundations — instrument-neutral light" "Light theme · unshipped candidate"

# Matched baseline/candidate comparison matrix. Every pair shares body markup,
# component styles, theme, and viewport; only the candidate token scope (and
# the sanctioned large-identity treatment) differs.
bake_comparison foundations.body.html comparisons/foundations-baseline-dark.html comparisons/foundations-candidate-dark.html foundations dark
bake_comparison foundations.body.html comparisons/foundations-baseline-light.html comparisons/foundations-candidate-light.html foundations light
bake_comparison navigation-topbar.body.html comparisons/navigation-wide-baseline-dark.html comparisons/navigation-wide-candidate-dark.html navigation-wide dark
bake_comparison navigation-topbar.body.html comparisons/navigation-wide-baseline-light.html comparisons/navigation-wide-candidate-light.html navigation-wide light
bake_comparison comparison-navigation-narrow.body.html comparisons/navigation-narrow-baseline-dark.html comparisons/navigation-narrow-candidate-dark.html navigation-narrow dark
bake_comparison comparison-navigation-narrow.body.html comparisons/navigation-narrow-baseline-light.html comparisons/navigation-narrow-candidate-light.html navigation-narrow light
bake_comparison data-timeline.body.html comparisons/timeline-wide-baseline-dark.html comparisons/timeline-wide-candidate-dark.html timeline-wide dark
bake_comparison data-timeline.body.html comparisons/timeline-wide-baseline-light.html comparisons/timeline-wide-candidate-light.html timeline-wide light
bake_comparison data-attention.body.html comparisons/attention-baseline-dark.html comparisons/attention-candidate-dark.html attention dark
bake_comparison data-attention.body.html comparisons/attention-baseline-light.html comparisons/attention-candidate-light.html attention light
bake_comparison data-review-facts.body.html comparisons/review-facts-baseline-dark.html comparisons/review-facts-candidate-dark.html review-facts dark
bake_comparison data-review-facts.body.html comparisons/review-facts-baseline-light.html comparisons/review-facts-candidate-light.html review-facts light
bake_comparison data-diff.body.html comparisons/annotated-diff-baseline-dark.html comparisons/annotated-diff-candidate-dark.html annotated-diff dark
bake_comparison data-diff.body.html comparisons/annotated-diff-baseline-light.html comparisons/annotated-diff-candidate-light.html annotated-diff light
bake_comparison comparison-compact-density.body.html comparisons/compact-density-baseline-dark.html comparisons/compact-density-candidate-dark.html compact-density dark
bake_comparison comparison-compact-density.body.html comparisons/compact-density-baseline-light.html comparisons/compact-density-candidate-light.html compact-density light
bake_comparison comparison-large-identity.body.html comparisons/large-identity-baseline-dark.html comparisons/large-identity-candidate-dark.html large-identity dark
bake_comparison comparison-large-identity.body.html comparisons/large-identity-baseline-light.html comparisons/large-identity-candidate-light.html large-identity light

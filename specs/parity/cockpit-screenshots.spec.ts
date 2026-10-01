import { mkdirSync } from "node:fs";
import { pathToFileURL } from "node:url";
import type { FrameLocator, Page } from "@playwright/test";
import { expect, test } from "@playwright/test";

const mockup = process.env.COCKPIT_MOCKUP;
const outputDirectory = process.env.COCKPIT_SCREENSHOT_DIR;

if (!mockup || !outputDirectory) {
  throw new Error("COCKPIT_MOCKUP and COCKPIT_SCREENSHOT_DIR are required");
}

mkdirSync(outputDirectory, { recursive: true });

async function openContract(
  page: Page,
  fragment: "cockpit-desktop" | "cockpit-narrow",
) {
  await page.goto(`${pathToFileURL(mockup).toString()}#${fragment}`);
  const contract = page.locator(`#${fragment}`);
  await expect(contract).toBeVisible();
  const preview = contract.frameLocator("iframe").frameLocator("#codex-visualization");
  await expect(preview.locator("#zaplex-panes [data-pane]").first()).toBeVisible();
  await expect(preview.locator("#zaplex-panes svg.lucide").first()).toBeVisible();
  return { contract, preview };
}

async function expectWaitingPulse(preview: FrameLocator, reducedMotion = false) {
  const waitingDot = preview.locator(".zp-dot.wait").first();
  await expect(waitingDot).toBeVisible();
  const pulse = await waitingDot.evaluate((element) => {
    const core = getComputedStyle(element);
    const ring = getComputedStyle(element, "::after");
    const animation = element.getAnimations({ subtree: true }).find(
      (candidate) => candidate instanceof CSSAnimation && candidate.animationName === "zp-wait-ring",
    );
    const frames = animation?.effect instanceof KeyframeEffect
      ? animation.effect.getKeyframes().map((frame) => ({
        offset: frame.computedOffset,
        scale: new DOMMatrix(String(frame.transform)).a,
        opacity: Number(frame.opacity),
      }))
      : [];
    return {
      coreAnimation: core.animationName,
      coreDuration: core.animationDuration,
      content: ring.content,
      position: ring.position,
      borderStyle: ring.borderStyle,
      borderWidth: ring.borderWidth,
      borderColor: ring.borderColor,
      animation: ring.animationName,
      duration: ring.animationDuration,
      iterations: ring.animationIterationCount,
      scale: new DOMMatrix(ring.transform).a,
      opacity: Number(ring.opacity),
      coreWidth: Number.parseFloat(core.width),
      ringWidth: Number.parseFloat(ring.width),
      frames,
    };
  });
  // An absent pseudo-element also reports animation:none; require a real visible ring.
  expect(pulse.content).toBe('""');
  expect(pulse.position).toBe("absolute");
  expect(pulse.borderStyle).toBe("solid");
  expect(pulse.borderWidth).toBe("1px");
  expect(pulse.borderColor).not.toBe("rgba(0, 0, 0, 0)");
  expect(pulse.coreWidth).toBeGreaterThan(0);
  expect(pulse.ringWidth).toBe(pulse.coreWidth);
  if (reducedMotion) {
    expect(pulse.coreAnimation).toBe("none");
    expect(pulse.animation).toBe("none");
    expect(pulse.frames).toEqual([]);
    expect(pulse.scale).toBeCloseTo(1.45);
    expect(pulse.opacity).toBeCloseTo(0.36);
  } else {
    expect(pulse.coreAnimation).toBe("zp-wait-core");
    expect(pulse.coreDuration).toBe("1.6s");
    expect(pulse.animation).toBe("zp-wait-ring");
    expect(pulse.duration).toBe("1.6s");
    expect(pulse.iterations).toBe("infinite");
    expect(pulse.frames).toEqual([
      { offset: 0, scale: 1, opacity: 0.58 },
      { offset: 1, scale: 2, opacity: 0 },
    ]);
  }
  const otherStates = await preview.locator(".zp-dot:not(.wait)").evaluateAll((elements) =>
    elements.map((element) => ({
      animation: getComputedStyle(element).animationName,
      ringAnimation: getComputedStyle(element, "::after").animationName,
      ringContent: getComputedStyle(element, "::after").content,
    })),
  );
  expect(otherStates.length).toBeGreaterThan(0);
  for (const state of otherStates) {
    expect(state).toEqual({ animation: "none", ringAnimation: "none", ringContent: "none" });
  }
}

test("normal Cockpit contract", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.setViewportSize({ width: 1440, height: 1000 });
  const { preview } = await openContract(page, "cockpit-desktop");
  await expectWaitingPulse(preview);
  await preview.locator("#zaplex-panes").screenshot({
    animations: "disabled",
    path: `${outputDirectory}/cockpit-desktop.png`,
  });
});

test("narrow Cockpit contract", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.setViewportSize({ width: 760, height: 900 });
  const { preview } = await openContract(page, "cockpit-narrow");
  await expectWaitingPulse(preview);
  await preview.locator("#zaplex-panes").screenshot({
    animations: "disabled",
    path: `${outputDirectory}/cockpit-narrow.png`,
  });
});

test("reduced-motion Cockpit contract", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1440, height: 1000 });
  const { preview } = await openContract(page, "cockpit-desktop");
  await expectWaitingPulse(preview, true);
  await preview.locator("#zaplex-panes").screenshot({
    animations: "disabled",
    path: `${outputDirectory}/cockpit-reduced-motion.png`,
  });
});

test("same interactive reference backs both viewport examples", async ({ page }) => {
  await page.goto(pathToFileURL(mockup).toString());
  for (const id of ["cockpit-desktop", "cockpit-narrow"]) {
    await expect(page.locator(`#${id} iframe`)).toHaveAttribute("src", "premium-workspace.html");
  }
  const { preview } = await openContract(page, "cockpit-desktop");
  await expect(preview.locator("[data-pane]")).toHaveCount(3);
  await expect(preview.getByText("Shell-Sessions in Verbindungen öffnen")).toHaveCount(0);
  await preview.locator("[data-menu]").click();
  await preview.locator("[data-more]").first().click();
  await expect(preview.locator("[data-launch-menu]")).toBeVisible();
  await expect(preview.locator("[data-flyout]")).toBeVisible();
});

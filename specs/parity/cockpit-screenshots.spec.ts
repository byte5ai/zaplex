import { mkdirSync } from "node:fs";
import { pathToFileURL } from "node:url";
import type { Page } from "@playwright/test";
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

test("normal Cockpit contract", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  const { preview } = await openContract(page, "cockpit-desktop");
  await preview.locator("#zaplex-panes").screenshot({
    animations: "disabled",
    path: `${outputDirectory}/cockpit-desktop.png`,
  });
});

test("narrow Cockpit contract", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 900 });
  const { preview } = await openContract(page, "cockpit-narrow");
  await preview.locator("#zaplex-panes").screenshot({
    animations: "disabled",
    path: `${outputDirectory}/cockpit-narrow.png`,
  });
});

test("reduced-motion Cockpit contract", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1440, height: 1000 });
  const { preview } = await openContract(page, "cockpit-desktop");
  const waitingDot = preview.locator(".zp-dot.wait").first();
  await expect(waitingDot).toBeVisible();
  const animationName = await waitingDot.evaluate((element) =>
    getComputedStyle(element, "::after").animationName,
  );
  expect(animationName).toBe("none");
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

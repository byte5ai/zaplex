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
  return contract;
}

test("normal Cockpit contract", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  const contract = await openContract(page, "cockpit-desktop");
  await contract.screenshot({
    animations: "disabled",
    path: `${outputDirectory}/cockpit-desktop.png`,
  });
});

test("narrow Cockpit contract", async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 900 });
  const contract = await openContract(page, "cockpit-narrow");
  await contract.screenshot({
    animations: "disabled",
    path: `${outputDirectory}/cockpit-narrow.png`,
  });
});

test("reduced-motion Cockpit contract", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1440, height: 1000 });
  const contract = await openContract(page, "cockpit-desktop");
  const waitingDot = page.locator(".tree .state-dot.waiting").first();
  await expect(waitingDot).toBeVisible();
  const animationName = await waitingDot.evaluate((element) =>
    getComputedStyle(element, "::after").animationName,
  );
  expect(animationName).toBe("none");
  await contract.screenshot({
    animations: "disabled",
    path: `${outputDirectory}/cockpit-reduced-motion.png`,
  });
});

import { test, expect } from "@playwright/test";

test("settings route", async ({ page }) => {
  await page.goto("/settings?tab=profile#security");
  await expect(page.getByText("Settings")).toBeVisible();
});

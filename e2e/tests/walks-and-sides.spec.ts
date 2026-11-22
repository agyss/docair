import { expect, test } from '@playwright/test';
import { openStatic } from './helpers';

/***
 * The page's frame: the walks are a header button next to Tests with a panel of their own, and the side panels' widths are dragged.
 *
 * @id      e2e.walks-and-sides
 * @kind    test
 * @part-of e2e
 ***/
test.describe('walks and side panels', () => {
  /***
   * Walkthroughs sits next to Tests and opens the walks; there is no footer; the walk on screen is marked when the panel opens again.
   *
   * @part-of e2e.walks-and-sides
   * @covers  docair-viewer.page_walks.walk_list "checks the button next to Tests and the panel of walks"
   ***/
  test('the walks are a button next to Tests', async ({ page }) => {
    await openStatic(page, 'shop');
    const walks = page.getByTestId('walks-button');
    await expect(walks).toContainText('Walkthroughs');
    const nextToTests = await page.evaluate(() => document.querySelector('[data-test="tests-toggle"]')?.nextElementSibling?.id);
    expect(nextToTests).toBe('btn-walks');
    await expect(page.locator('#flow')).toHaveCount(0);
    await walks.click();
    const list = page.getByTestId('walks-list');
    await expect(list.locator('a[href="#slice/flow:checkout"]')).toBeVisible();
    await list.locator('a[href="#slice/flow:checkout"]').click();
    await expect(page).toHaveURL(/#slice\/flow:checkout$/);
    await walks.click();
    await expect(page.locator('.tour.on')).toHaveAttribute('data-walk', 'slice/flow:checkout');
  });

  /***
   * Each side panel is dragged wider at its inner edge, keeps its width after a reload, and a double-click puts it back.
   *
   * @part-of e2e.walks-and-sides
   * @covers  docair-viewer.doc-page.wire-sides "drags, keeps and resets the widths"
   ***/
  test('the side panels are dragged wider and come back', async ({ page }) => {
    await page.setViewportSize({ width: 1500, height: 900 });
    await openStatic(page, 'shop');
    const width = (sel: string) => page.locator(sel).evaluate((e) => Math.round(e.getBoundingClientRect().width));
    const drag = async (handle: string, dx: number) => {
      const h = (await page.getByTestId(handle).boundingBox())!;
      await page.mouse.move(h.x + h.width / 2, h.y + 100);
      await page.mouse.down();
      await page.mouse.move(h.x + h.width / 2 + dx, h.y + 100, { steps: 4 });
      await page.mouse.up();
    };
    const [details, tree] = [await width('#details'), await width('#tree')];
    await drag('details-handle', -150);
    await drag('tree-handle', 100);
    expect(Math.abs((await width('#details')) - (details + 150))).toBeLessThan(6);
    expect(Math.abs((await width('#tree')) - (tree + 100))).toBeLessThan(6);
    await page.reload();
    await expect(page.locator('body[data-ready="true"]')).toBeAttached();
    expect(Math.abs((await width('#details')) - (details + 150))).toBeLessThan(6);
    await page.getByTestId('details-handle').dblclick();
    await page.getByTestId('tree-handle').dblclick();
    expect(await width('#details')).toBe(details);
    expect(await width('#tree')).toBe(tree);
  });

  /***
   * The header's ◧ and ◨ hide the tree and the details, the picture takes the room, the choice stays after a reload, and a second click shows the panel again.
   *
   * @part-of e2e.walks-and-sides
   * @covers  docair-viewer.doc-page.wire-sides "hides and shows the side panels"
   ***/
  test('a side panel can be hidden and shown again', async ({ page }) => {
    await page.setViewportSize({ width: 1500, height: 900 });
    await openStatic(page, 'shop');
    const width = (sel: string) => page.locator(sel).evaluate((e) => Math.round(e.getBoundingClientRect().width));
    const graph = await width('#graph');
    await page.getByTestId('hide-details').click();
    await expect(page.locator('#details')).toBeHidden();
    await expect(page.getByTestId('hide-details')).toHaveAttribute('aria-pressed', 'true');
    expect(await width('#graph')).toBeGreaterThan(graph + 200);
    await page.getByTestId('hide-tree').click();
    await expect(page.locator('#tree')).toBeHidden();
    await page.reload();
    await expect(page.locator('body[data-ready="true"]')).toBeAttached();
    await expect(page.locator('#details')).toBeHidden();
    await expect(page.locator('#tree')).toBeHidden();
    await page.getByTestId('hide-details').click();
    await page.getByTestId('hide-tree').click();
    await expect(page.locator('#details')).toBeVisible();
    await expect(page.locator('#tree')).toBeVisible();
    expect(await width('#graph')).toBe(graph);
  });
});

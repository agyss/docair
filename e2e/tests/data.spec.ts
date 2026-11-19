import { expect, test } from '@playwright/test';
import { graphOf, openStatic, pictured } from './helpers';

/***
 * Code and data: parts are drawn where they run; data has its own list and can be followed along its way.
 *
 * @id      e2e.data
 * @kind    test
 * @part-of e2e
 ***/
test.describe('code and data', () => {
  /***
   * A part whose code a server delivers but which runs in the browser is placed in the browser, and says where its code comes from.
   *
   * @part-of e2e.data
   * @covers  docair-core.build.build "checks @runs-in places the part where it runs"
   * @covers  docair-viewer.page_panels.panel-graph.details "checks the details say where it runs and where its code comes from"
   ***/
  test('the shop front end is drawn in the browser, where it runs', async ({ page }) => {
    const node = graphOf('shop').nodes.find((n: any) => n.id === 'shop-ui');
    expect(node.part_of).toBe('browser');
    expect(node.code_from).toBe('shop');
    await openStatic(page, 'shop', '#part/shop-ui');
    await expect(page.locator('#details')).toContainText('runs in');
    await expect(page.locator('#details')).toContainText('code from');
  });

  /***
   * Data is not a box among the parts; the Data list says who makes it and how many lines carry it, and Follow walks its way, step by step, in a picture of its own.
   *
   * @part-of e2e.data
   * @covers  docair-viewer.page_glossary.glossary "checks the data list"
   * @covers  docair-core.slice.slice "checks Follow walks the data's way"
   * @covers  docair-core.slice.messages "checks each step's messages"
   * @covers  docair-viewer.doc-page.walk "checks the walk's steps and the lines that carry the datum"
   ***/
  test('the order is data: made by the cart, followed along its way', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/code');
    expect(await pictured(page)).not.toContain('data.order');
    await page.getByTestId('data-button').click();
    const list = page.getByTestId('data-list');
    await expect(list).toContainText('order');
    await expect(list).toContainText('2 lines');
    await expect(list).toContainText('made by Cart');
    await list.getByTestId('follow').first().click();
    await expect(page).toHaveURL(/#slice\/data\.order$/);
    await expect(page.getByTestId('follow-bar')).toContainText('order');
    const card = page.getByTestId('step-card');
    await expect(card).toContainText('step 1 of 4');
    await expect(page.getByTestId('carried')).toHaveText('Carried by 2 lines, all in this walk.');
    await expect(card.locator('h3')).toHaveText('Cart makes ◇ order');
    const next = card.locator('[data-step="1"]');
    await next.click();
    await expect(card.locator('h3')).toHaveText('Cart calls create_order');
    await expect(card.locator('.hop-carry .data-ref').first()).toHaveText('CreateOrder');
    await next.click();
    await expect(card.locator('h3')).toHaveText('create_order writes to orders-db');
    // The call carries the order there, and its answer brings it back.
    await next.click();
    await expect(card.locator('h3')).toHaveText('create_order answers Cart');
    await expect(card.locator('.hop.return')).toHaveCount(1);
    await expect(next).toBeDisabled();
    await page.getByTestId('unfollow').click();
    await expect(page.getByTestId('follow-bar')).toBeHidden();
    await expect(page).not.toHaveURL(/#slice\//);
  });

  /***
   * Follow opens at the datum's first step, at a size to read: the camera shows the step at 0.8 to 1.3 screen pixels per unit, and a slice is laid out without shrinking what its boxes hold, so every word in view is 8 px or more.
   *
   * @part-of e2e.data
   * @covers  docair-viewer.layout_input.options_for "checks a slice keeps its words at their size"
   * @covers  docair-viewer.doc-page.walk "checks Follow opens at the first step's camera"
   ***/
  test("Follow opens at the datum's first step, at a size to read", async ({ page }) => {
    await openStatic(page, 'shop', '#slice/data.order');
    await expect(page.getByTestId('step-card')).toContainText('step 1 of 4');
    // The camera comes to rest on the first step.
    let view = '';
    await expect
      .poll(async () => {
        const was = view;
        view = (await page.locator('#pic svg.docair-pic').getAttribute('viewBox')) || '';
        return was === view;
      })
      .toBe(true);
    const r = await page.evaluate(() => {
      const pic = document.querySelector('#pic')!.getBoundingClientRect();
      const svg = document.querySelector('#pic svg.docair-pic') as SVGSVGElement;
      const sizes = [...svg.querySelectorAll('text')]
        .filter((t) => {
          const b = t.getBoundingClientRect();
          return b.width > 0 && b.right > pic.left && b.left < pic.right && b.bottom > pic.top && b.top < pic.bottom;
        })
        .map((t) => parseFloat(getComputedStyle(t).fontSize) * (t as SVGGraphicsElement).getScreenCTM()!.a);
      return { scale: svg.getScreenCTM()!.a, words: sizes.length, smallest: Math.min(...sizes) };
    });
    expect(r.scale).toBeGreaterThanOrEqual(0.79);
    expect(r.scale).toBeLessThanOrEqual(1.31);
    expect(r.words).toBeGreaterThan(3);
    expect(r.smallest).toBeGreaterThanOrEqual(8);
  });

  /***
   * A datum a function makes goes back to its caller in the answer, and on from there; a line the walk cannot reach from where the datum is made says so; the walk's own parts do not fade while it plays.
   *
   * @part-of e2e.data
   * @covers  docair-core.slice.slice "checks data is followed back along answers, and on"
   * @covers  docair-core.slice.messages "checks a step of a data walk is the answer that brings it back"
   * @covers  docair-viewer.doc-page.walk "checks the walk's own parts do not fade while it plays"
   ***/
  test('data that a function makes and returns is followed back to its caller, and on', async ({ page }) => {
    await openStatic(page, 'answer', '#slice/data.summary');
    const card = page.getByTestId('step-card');
    const next = card.locator('[data-step="1"]');
    // The first card says what the walk covers.
    await expect(page.getByTestId('carried')).toContainText('Carried by 2 lines, all in this walk; 1 of them starts a stretch of its own');
    const titles: string[] = [];
    for (let k = 0; k < 4; k++) {
      titles.push((await card.locator('h3').textContent()) ?? '');
      if (k === 1) {
        // The answer goes back as an answer, and the parts of the step do not fade.
        await expect(card.locator('.hop.return')).toHaveCount(1);
        await expect(page.locator('#pic g.part[data-id="app.summarize"]')).not.toHaveClass(/flow-dim/);
        await expect(page.locator('#pic g.part[data-id="app.ask"]')).not.toHaveClass(/flow-dim/);
      }
      if (k < 3) await next.click();
    }
    expect(titles).toEqual(['summarize makes ◇ summary', 'summarize answers ask', 'ask calls model', 'report calls mail']);
    await expect(card).toContainText('Not reached from where ◇ summary is made');
    await openStatic(page, 'answer', '#slice/data.answer');
    const answer: string[] = [];
    for (let k = 0; k < 3; k++) {
      answer.push((await page.getByTestId('step-card').locator('h3').textContent()) ?? '');
      if (k < 2) await page.getByTestId('step-card').locator('[data-step="1"]').click();
    }
    expect(answer).toEqual(['model makes ◇ answer', 'model answers ask', 'ask answers asker']);
  });
});

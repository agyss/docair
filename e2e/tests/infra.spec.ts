import { expect, test } from '@playwright/test';
import { graphOf, openStatic, pictured } from './helpers';

/***
 * Infrastructure read from its code: Terraform, a Helm chart and Kubernetes manifests without one doc block.
 *
 * @id      e2e.infra
 * @kind    test
 * @part-of e2e
 ***/
test.describe('how things are provisioned and routed', () => {
  /***
   * Terraform creates the cluster and the releases, a release holds its chart's objects, and requests go Traefik → Ingress → Service → Deployment.
   *
   * @part-of e2e.infra
   * @covers  docair-infra.index.discover "checks resources and objects become parts"
   * @covers  docair-infra.index.anchors "checks creates, routes and selects"
   * @covers  docair-core.layer_view.picture "checks the infrastructure, Kubernetes and network layers draw them"
   ***/
  test('Terraform, Helm and manifests become one picture', async ({ page }) => {
    const graph = graphOf('platform');
    const partOf = (id: string) => graph.nodes.find((n: any) => n.id === id)?.part_of;
    expect(partOf('k8s.release.shop')).toBe('k8s.ns.shop');
    expect(partOf('k8s.ns.shop')).toBe('k8s.cluster.dev');
    expect(partOf('k8s.deploy.api')).toBe('k8s.release.shop');
    const edges = graph.edges.map((e: any) => `${e.kind} ${e.from} -> ${e.to}`);
    expect(edges).toEqual(
      expect.arrayContaining([
        'create tf.kind_cluster.dev -> k8s.cluster.dev',
        'create tf.helm_release.shop -> k8s.release.shop',
        'data tf.helm_release.ingress -> tf.helm_release.shop',
        'data k8s.release.traefik -> k8s.ingress.shop',
        'route k8s.ingress.shop -> k8s.svc.api',
        'select k8s.svc.api -> k8s.deploy.api',
      ]),
    );
    await openStatic(page, 'platform', '#layer/infra');
    expect(await pictured(page)).toEqual(expect.arrayContaining(['tf.kind_cluster.dev', 'tf.helm_release.shop']));
    await openStatic(page, 'platform', '#layer/k8s');
    expect(await pictured(page)).toEqual(expect.arrayContaining(['k8s.release.shop', 'k8s.deploy.api']));
    await openStatic(page, 'platform', '#part/k8s.ingress.shop');
    await expect(page.locator('#details')).toContainText('HTTP /api, port 80');
    await expect(page.locator('#details')).toContainText('requests (ingress class traefik)');
    await expect(page.locator('#details')).toContainText('discovered');
  });
});

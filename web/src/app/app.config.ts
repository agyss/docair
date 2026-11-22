import { ApplicationConfig, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideHttpClient, withXhr } from '@angular/common/http';

/***
 * The app's providers: global error listeners and HttpClient. HttpClient uses
 * XHR here (`withXhr`; Angular's default is fetch), because only XHR reports
 * how much of an upload has been sent.
 *
 * @id      docair-web.config
 * @kind    config
 * @part-of docair-web
 ***/
export const appConfig: ApplicationConfig = {
  providers: [provideBrowserGlobalErrorListeners(), provideHttpClient(withXhr())],
};

import { ApplicationConfig, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideHttpClient } from '@angular/common/http';

/***
 * The app's providers: global error listeners and HttpClient. HttpClient uses
 * XHR here (not fetch), because only XHR reports upload progress.
 *
 * @id      docair-web.config
 * @kind    config
 * @part-of docair-web
 ***/
export const appConfig: ApplicationConfig = {
  providers: [provideBrowserGlobalErrorListeners(), provideHttpClient()],
};

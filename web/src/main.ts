import { bootstrapApplication } from '@angular/platform-browser';
import { appConfig } from './app/app.config';
import { App } from './app/app';

/***
 * Start the Angular app in the browser.
 *
 * @id      docair-web.main
 * @kind    function
 * @part-of docair-web
 * @calls   docair-web.app () -> page
 * @flow    open 3 "the browser starts the Angular app"
 ***/
bootstrapApplication(App, appConfig).catch((err) => console.error(err));

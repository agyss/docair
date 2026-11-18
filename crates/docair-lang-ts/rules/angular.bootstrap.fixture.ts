// Fixture for angular.bootstrap.yaml. Every `//> ` line is a fact the rule must
// find, and it must find no others.

export class App {}

bootstrapApplication(App, appConfig).catch((err) => console.error(err));

// Another function's call is not Angular's.
start(App);

//> calls class App from bootstrapApplication

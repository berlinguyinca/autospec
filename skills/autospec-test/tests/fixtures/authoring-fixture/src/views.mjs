// authoring-fixture component source — the "app source" selectors resolve against.
//
// Deliberate traps embedded for the Stage 2A integration test (spec §10):
//   - trap1: the Products heading has NO test id at all, so an author who invents
//     one for it must be caught by PW_SELECTOR_UNVERIFIED.
//   - trap2: the literal text "Orders" appears BOTH in the nav rail and as the
//     Orders page heading (an unscoped getByText('Orders') is a PW_STRICT_MODE_RISK).
//   - trap3 lives in the server (delete returns 200 but the row persists).
//
// These are real HTML-producing functions; the integration test serves them over
// real HTTP and the selector-evidence resolver greps THIS file for evidence.

/** Shared nav rail — note the "Orders" link text (half of the strict-mode trap). */
export function navRail() {
    return `
    <nav data-testid="nav-rail">
      <a href="/products">Products</a>
      <a href="/orders">Orders</a>
      <a href="/account">Account</a>
    </nav>`;
}

/** /products — heading has NO data-testid (trap1). */
export function productsPage(rows) {
    const items = rows
        .map(
            (r) =>
                `<li data-testid="product-row-${r.id}">${r.name}` +
                `<button data-testid="delete-product-${r.id}">Delete</button></li>`
        )
        .join('');
    return page(
        // intentionally no data-testid on this <h1>
        `<h1>Products</h1>
         <ul data-testid="product-list">${items}</ul>`
    );
}

/** /orders — heading text "Orders" collides with the nav-rail link text (trap2). */
export function ordersPage() {
    return page(`<h1 data-testid="orders-heading">Orders</h1>
      <p data-testid="orders-empty">No orders yet.</p>`);
}

/** /account — a clean route with a verifiable data-testid. */
export function accountPage() {
    return page(`<h1 data-testid="account-heading">Account</h1>
      <button data-testid="save-account">Save changes</button>`);
}

function page(body) {
    return `<!doctype html><html><head><title>authoring-fixture</title></head>
      <body>${navRail()}<main>${body}</main></body></html>`;
}

import { Logger } from '../logger';
import type { Order, OrderRequest } from '../types';

export interface ReconciliationResult {
  missingOnExchange: OrderRequest[];
  unexpectedOnExchange: Order[];
  matched: number;
}

export class OrderReconciliation {
  private readonly logger = new Logger('OrderReconciliation');

  compare(adapterOrders: Order[], desiredOrders: OrderRequest[]): ReconciliationResult {
    const adapterByKey = new Map(adapterOrders.map((order) => [this.orderKey(order.side, order.price, order.qty), order]));
    const desiredByKey = new Map(desiredOrders.map((order) => [this.orderKey(order.side, order.price, order.qty), order]));

    const missingOnExchange = desiredOrders.filter((order) => !adapterByKey.has(this.orderKey(order.side, order.price, order.qty)));
    const unexpectedOnExchange = adapterOrders.filter((order) => !desiredByKey.has(this.orderKey(order.side, order.price, order.qty)));
    const matched = desiredOrders.length - missingOnExchange.length;

    if (missingOnExchange.length || unexpectedOnExchange.length) {
      this.logger.warn('Order reconciliation mismatch detected.', {
        missingOnExchange: missingOnExchange.map((order) => this.orderKey(order.side, order.price, order.qty)),
        unexpectedOnExchange: unexpectedOnExchange.map((order) => this.orderKey(order.side, order.price, order.qty)),
        matched
      });
    } else {
      this.logger.debug('Order reconciliation clean.', { matched });
    }

    return { missingOnExchange, unexpectedOnExchange, matched };
  }

  private orderKey(side: string, price: number | undefined, qty: number): string {
    return `${side}:${price ?? 'mkt'}:${qty}`;
  }
}

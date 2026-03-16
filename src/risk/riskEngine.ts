import type { AppConfig } from '../config';
import type { Position, RiskDecision } from '../types';

export class RiskEngine {
  private readonly shortOnlyLongFlipEpsilon: number;

  constructor(private readonly config: AppConfig) {
    const orderSizeText = String(config.orderSize);
    const decimals = orderSizeText.includes('.') ? orderSizeText.split('.')[1]?.length ?? 0 : 0;
    this.shortOnlyLongFlipEpsilon = decimals <= 0 ? 1e-9 : 10 ** (-(decimals + 3));
  }

  validateNewOrder(position: Position, side: 'buy' | 'sell', qty: number): RiskDecision {
    if (this.config.killSwitch) {
      return { ok: false, reason: 'kill_switch_enabled' };
    }
    if (qty <= 0) {
      return { ok: false, reason: 'qty_must_be_positive' };
    }
    const projected = position.size + (side === 'buy' ? qty : -qty);
    if (this.config.gridMode === 'short_only' && projected > this.shortOnlyLongFlipEpsilon) {
      return { ok: false, reason: 'short_only_long_flip_blocked' };
    }
    if (Math.abs(projected) > this.config.maxPositionAbs) {
      return { ok: false, reason: 'max_position_breached' };
    }
    return { ok: true };
  }
}

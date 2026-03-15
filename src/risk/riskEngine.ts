import type { AppConfig } from '../config';
import type { Position, RiskDecision } from '../types';

export class RiskEngine {
  constructor(private readonly config: AppConfig) {}

  validateNewOrder(position: Position, side: 'buy' | 'sell', qty: number): RiskDecision {
    if (this.config.killSwitch) {
      return { ok: false, reason: 'kill_switch_enabled' };
    }
    const projected = position.size + (side === 'buy' ? qty : -qty);
    if (Math.abs(projected) > this.config.maxPositionAbs) {
      return { ok: false, reason: 'max_position_breached' };
    }
    if (qty <= 0) {
      return { ok: false, reason: 'qty_must_be_positive' };
    }
    return { ok: true };
  }
}

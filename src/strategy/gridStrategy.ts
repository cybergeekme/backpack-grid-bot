import type { AppConfig } from '../config';
import type { GridLevel, StrategySnapshot } from '../types';

export class GridStrategyEngine {
  constructor(private readonly config: AppConfig) {}

  buildGrid(midPrice: number): StrategySnapshot {
    const levels: GridLevel[] = [];
    for (let i = 1; i <= this.config.levels; i += 1) {
      const ratio = (this.config.spacingBps * i) / 10_000;
      levels.push({
        index: i,
        side: 'buy',
        price: round(midPrice * (1 - ratio)),
        qty: this.config.orderSize
      });
      levels.push({
        index: i,
        side: 'sell',
        price: round(midPrice * (1 + ratio)),
        qty: this.config.orderSize
      });
    }
    return {
      symbol: this.config.symbol,
      midPrice: round(midPrice),
      levels: levels.sort((a, b) => a.price - b.price)
    };
  }
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}

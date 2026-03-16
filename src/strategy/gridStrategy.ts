import type { AppConfig } from '../config';
import type { GridLevel, StrategySnapshot } from '../types';

export class GridStrategyEngine {
  constructor(private readonly config: AppConfig) {}

  buildGrid(midPrice: number): StrategySnapshot {
    const bounded = this.buildBoundedGrid(midPrice);
    if (bounded) return bounded;

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

  private buildBoundedGrid(midPrice: number): StrategySnapshot | undefined {
    const minPrice = this.config.gridMinPrice;
    const maxPrice = this.config.gridMaxPrice;
    if (minPrice === undefined || maxPrice === undefined) return undefined;
    if (!(minPrice > 0 && maxPrice > minPrice)) {
      throw new Error(`Invalid bounded grid range: min=${minPrice} max=${maxPrice}`);
    }

    const levels: GridLevel[] = [];
    const slices = this.config.levels + 1;
    const step = (maxPrice - minPrice) / slices;
    for (let i = 1; i <= this.config.levels; i += 1) {
      const price = round(minPrice + step * i);
      if (price === round(midPrice)) continue;
      levels.push({
        index: i,
        side: price < midPrice ? 'buy' : 'sell',
        price,
        qty: this.config.orderSize
      });
    }

    return {
      symbol: this.config.symbol,
      midPrice: round(midPrice),
      levels: levels.sort((a, b) => a.price - b.price),
      range: {
        minPrice: round(minPrice),
        maxPrice: round(maxPrice)
      }
    };
  }
}

function round(value: number): number {
  return Math.round(value * 100) / 100;
}

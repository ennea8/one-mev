# one-mev

Generally, there are three kinds of MEV: sandwich, arbitrage, and liquidation.

This repo implements a type of arbitrage by backrunning Uniswap V2/Uniswap V3 transactions on Ethereum.

This does not promise profitability, as other factors such as node speed, swap paths, bribes, etc., also matter. However, it should serve as an example for study purposes.

Use at your own risk.


## Acknowledgements

- [artemis](https://github.com/paradigmxyz/artemis)
- [alloy](https://github.com/alloy-rs/alloy)
- [rusty-sando](https://github.com/mouseless-eth/rusty-sando.git)
- [sandooo](https://github.com/solidquant/sandooo.git)
- [revm](https://github.com/revm)
- [foundry](https://github.com/foundry-rs/foundry)
- [reth](https://github.com/paradigmxyz/reth)
- [ethers-rs](https://github.com/gakonst/ethers-rs)
- [ethers-flashbots](https://github.com/onbjerg/ethers-flashbots)
- [alloy-mev](https://github.com/leruaa/alloy-mev)
- [revm-by-example](https://github.com/Cionn3/revm-by-example/tree/master/src)
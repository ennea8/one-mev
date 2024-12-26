use alloy_sol_types::sol;

sol! {

  // ------------------------------------------------------------------------
  // interface IOneSimulator

    #[derive(Debug)]
    struct SwapParams {
        uint8 protocol;
        address handler;
        address tokenIn;
        address tokenOut;
        uint24 fee;
        uint256 amount;
    }

    function simulateSwapIn(
        SwapParams[] calldata paramsArray
    ) external returns (uint256 amountIn, uint256 amountOut, int256 profit);
    function simulateUniswapV2SwapIn(
        SwapParams memory params
    ) external returns (uint256 amountOut);
    function simulateUniswapV3SwapIn(
        SwapParams memory params
    ) external returns (uint256 amountOut);

    // ------------------------------------------------------------------------
    // interface IOne
    function arbitrage(bytes calldata pathArrayData, address baseToken, bool requireProfit) external returns (int256 profit);

    function arbitrageMulti(bytes calldata pathArrayData, uint256[] calldata groupSizeArr, bool requireProfit) external returns (int256 profit);

    function swap(bytes calldata pathArrayData, uint256 stepTotal, uint256 minAmountOut) public returns (uint256 amountOut);

    function swapMulti(
        bytes calldata pathArrayData,
        uint256[] calldata groupSizeArr,
        address[] calldata minTokenArr,
        uint256[] calldata minBalanceArr
    ) external returns (uint256[] memory amountOutArr);

    // ------------------------------------------------------------------------
    // events from dex protocol

    event UniswapV3Swap(
        address indexed sender,
        address indexed recipient,
        int256 amount0,
        int256 amount1,
        uint160 sqrtPriceX96,
        uint128 liquidity,
        int24 tick
    );

    event UniswapV2Swap(
        address indexed sender,
        uint amount0In,
        uint amount1In,
        uint amount0Out,
        uint amount1Out,
        address indexed to
    );

}

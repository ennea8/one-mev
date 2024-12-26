use alloy::sol;
use alloy_sol_types::{SolCall, SolValue};

sol! {
  #[sol(rpc)]
  contract ERC20 {
      function balanceOf(address owner) external view returns (uint256 balance);
      function approve(address spender, uint256 amount) external returns (bool);
      function transfer(address recipient, uint256 amount) external returns (bool);
      function transferFrom(address from, address recipient, uint256 amount) external returns (bool);
      function allowance(address owner, address spender) external view returns (uint256);
      function name() external view returns (string memory);
      function symbol() external view returns (string memory);
      function decimals() external view returns (uint8);
      function totalSupply() external view returns (uint256);
      function deposit() external payable;
      function withdraw(uint256 amount) external;
}
}

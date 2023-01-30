use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use lazy_static::lazy_static;

const POOL_VALIDATOR_BYTES: &str =
    "199d062704000400040204020404040404060406040804080404040204000400040204020601010400040a05000500\
    04040e20a20a53f905f41ebdd71c2c239f270392d0ae0f23f6bd9f3687d166eea745bbf604000402050004020402040\
    60500050005feffffffffffffffff010502050005000402050005000100d820d601b2a5730000d602db63087201d603\
    db6308a7d604b27203730100d605e4c6a70410d606e4c6a70505d607e4c6a70605d608b27202730200d609b27203730\
    300d60ab27202730400d60bb27203730500d60cb27202730600d60db27203730700d60e8c720d01d60fb27202730800\
    d610b27203730900d6118c721001d6128c720b02d613998c720a027212d614998c720c028c720d02d615b27205730a0\
    0d6169a99a37215730bd617b27205730c00d6189d72167217d61995919e72167217730d9a7218730e7218d61ab27205\
    730f00d61b7e721a05d61c8c721002d61d998c720f02721cd61e8c720902d61f9d7206721bd6207310d1ededededed9\
    3b272027311007204ededed93e4c672010410720593e4c672010505720693e4c6720106057207928cc77201018cc7a7\
    0193c27201c2a7ededed938c7208018c720901938c720a018c720b01938c720c01720e938c720f01721193b17202731\
    2959172137313d802d6219c721399721ba273147e721905d622b2a5731500eded937213f07214937221f0721dededed\
    ed93cbc272227316938602720e7213b2db6308722273170093860272117221b2db63087222731800e6c67222040893e\
    4c67222050e8c720401958f72137319ededec929a997206721e72079c7e9995907219721a72199a721a731a731b0572\
    1f92a39a9a72159c721a7217b27205731c00937214f0721392721d95917219721a731d9c721499721ba2731e7e72190\
    5d804d621e4c672010704d62299721a7221d6237e722205d6249999731f721c9c9972127320722395ed917224732191\
    721d7322edededed90722199721973239099721e9c7223721f9a721f7207907ef0998c720802721e069a9d9c99997e7\
    21e069d9c7e7206067e7222067e721a0672207e721d067e7224067220937213732493721473257326";

const BUNDLE_VALIDATOR_BYTES: &str =
    "19b803160400040004040404040404020400040005000402040204000502040404000402050205000404040005fcff\
    ffffffffffffff010100d80ed601b2a5730000d602db63087201d603e4c6a7050ed604b2a4730100d605db63087204d\
    6068cb2720573020002d607998cb27202730300027206d608e4c6a70408d609db6308a7d60a8cb2720973040001d60b\
    b27209730500d60cb27209730600d60d8c720c02d60e8c720b02d1ed938cb27202730700017203959372077308d808d\
    60fb2a5e4e3000400d610b2a5e4e3010400d611db63087210d612b27211730900d613b2e4c672040410730a00d614c6\
    72010804d6157e99721395e67214e47214e4c67201070405d616b2db6308720f730b00eded93c2720fd07208ededede\
    deded93e4c672100408720893e4c67210050e720393c27210c2a7938602720a730cb27211730d00938c7212018c720b\
    019399720e8c72120299720e9c7215720d93b27211730e00720ced938c7216018cb27205730f0001927e8c721602069\
    d9c9c7e9de4c6720405057e721305067e720d067e999d720e720d7215067e997206731006958f7207731193b2db6308\
    b2a47312007313008602720a73147315";

const DEPOSIT_TEMPLATE_BYTES: &str =
    "d808d601b2a4730000d602db63087201d6037301d604b2a5730200d6057303d606c57201d607b2a5730400d6088cb2\
    db6308a773050002eb027306d1ededed938cb27202730700017203ed93c27204720593860272067308b2db630872047\
    30900ededededed93cbc27207730a93d0e4c672070408720593e4c67207050e72039386028cb27202730b00017208b2\
    db63087207730c009386028cb27202730d00019c72087e730e05b2db63087207730f0093860272067310b2db6308720\
    773110090b0ada5d90109639593c272097312c1720973137314d90109599a8c7209018c7209027315";

pub const REDEEM_VALIDATOR_BYTES: &str =
    "19ad020a040208cd02217daf90deb73bdf8b6709bb42093fdfaff6573fd47b630e2d3fdd4a8193a74d0e2001010101\
    010101010101010101010101010101010101010101010101010101010e2000000000000000000000000000000000000\
    0000000000000000000000000000005d00f04000e691005040004000e36100204a00b08cd0279be667ef9dcbbac55a0\
    6295ce870b07029bfcdb2dce28d959f2815b16f81798ea02d192a39a8cc7a701730073011001020402d19683030193a\
    38cc7b2a57300000193c2b2a57301007473027303830108cdeeac93b1a573040500050005a09c01d801d601b2a57300\
    00eb027301d1eded93c27201730293860273037304b2db6308720173050090b0ada5d90102639593c272027306c1720\
    273077308d90102599a8c7202018c7202027309";

const REDEEM_TEMPLATE_BYTES: &str =
    "d801d601b2a5730000eb027301d1eded93c27201730293860273037304b2db6308720173050090b0ada5d901026395\
    93c272027306c1720273077308d90102599a8c7202018c7202027309";

lazy_static! {
    pub static ref POOL_VALIDATOR: ErgoTree =
        ErgoTree::sigma_parse_bytes(&base16::decode(POOL_VALIDATOR_BYTES.as_bytes()).unwrap()).unwrap();
    pub static ref BUNDLE_VALIDATOR: ErgoTree =
        ErgoTree::sigma_parse_bytes(&base16::decode(BUNDLE_VALIDATOR_BYTES.as_bytes()).unwrap()).unwrap();
    pub static ref REDEEM_VALIDATOR: ErgoTree =
        ErgoTree::sigma_parse_bytes(&base16::decode(REDEEM_VALIDATOR_BYTES.as_bytes()).unwrap()).unwrap();
    pub static ref DEPOSIT_TEMPLATE: Vec<u8> = base16::decode(DEPOSIT_TEMPLATE_BYTES.as_bytes()).unwrap();
    pub static ref REDEEM_TEMPLATE: Vec<u8> = base16::decode(REDEEM_TEMPLATE_BYTES.as_bytes()).unwrap();
}

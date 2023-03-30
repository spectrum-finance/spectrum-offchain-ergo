use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;
use lazy_static::lazy_static;

const POOL_VALIDATOR_BYTES: &str =
    "19c0062904000400040204020404040404060406040804080404040204000400040204020601010400040a05000500\
    0404040204020e202045638fde5b28db0f08d3ebe28663bc21333348cd7679e11500931a7f907090040004020500040\
    2040204060500050005feffffffffffffffff010502050005000402050005000100d820d601b2a5730000d602db6308\
    7201d603db6308a7d604b27203730100d605e4c6a70410d606e4c6a70505d607e4c6a70605d608b27202730200d609b\
    27203730300d60ab27202730400d60bb27203730500d60cb27202730600d60db27203730700d60e8c720d01d60fb272\
    02730800d610b27203730900d6118c721001d6128c720b02d613998c720a027212d6148c720902d615b27205730a00d\
    6169a99a37215730bd617b27205730c00d6189d72167217d61995919e72167217730d9a7218730e7218d61ab2720573\
    0f00d61b7e721a05d61c9d7206721bd61d998c720c028c720d02d61e8c721002d61f998c720f02721ed6207310d1ede\
    dededed93b272027311007204ededed93e4c672010410720593e4c672010505720693e4c6720106057207928cc77201\
    018cc7a70193c27201c2a7ededed938c7208018c720901938c720a018c720b01938c720c01720e938c720f01721193b\
    172027312959172137313d802d6219c721399721ba273147e721905d622b2a5731500ededed929a997206721472079c\
    7e9995907219721a72199a721a7316731705721c937213f0721d937221f0721fedededed93cbc272227318938602720\
    e7213b2db6308722273190093860272117221b2db63087222731a00e6c67222060893e4c67222070e8c720401958f72\
    13731bededec929a997206721472079c7e9995907219721a72199a721a731c731d05721c92a39a9a72159c721a7217b\
    27205731e0093721df0721392721f95917219721a731f9c721d99721ba273207e721905d804d621e4c672010704d622\
    99721a7221d6237e722205d62499997321721e9c9972127322722395ed917224732391721f7324edededed907221997\
    2197325909972149c7223721c9a721c7207907ef0998c7208027214069a9d9c99997e7214069d9c7e7206067e722206\
    7e721a0672207e721f067e7224067220937213732693721d73277328";

const BUNDLE_VALIDATOR_BYTES: &str =
    "19c404220400040004040404040004020601010601000400050004020404040205feffffffffffffffff0104080502\
    040004020502040405020402040004000101010005000404040004060404040205fcffffffffffffffff010100d80dd\
    601b2a5730000d602db63087201d603e4c6a7070ed604b2a4730100d605db63087204d6068cb2720573020002d60799\
    8cb27202730300027206d608e4c6a70608d609db6308a7d60ab27209730400d60bb27205730500d60c7306d60d7307d\
    1ed938cb27202730800017203959372077309d80cd60eb2a5e4e3000400d60fb2a5e4e3010400d610b2e4c672040410\
    730a00d611c672010804d61299721095e67211e47211e4c672010704d6138cb27209730b0001d614db6308720fd615b\
    27209730c00d6168c721502d6177e721205d6189972169c72178c720a02d619999d9c99997e8c720b02069d9c7ee4c6\
    72040505067e7212067e721006720c7e7218067e9999730d8cb27205730e00029c997206730f721706720ceded93c27\
    20ed07208edededed93e4c6720f0608720893e4c6720f070e720393c2720fc2a7959172127310d801d61ab272147311\
    00eded93860272137312b27214731300938c721a018c721501939972168c721a02721893860272137314b2721473150\
    093b27214731600720a95917219720dd801d61ab2db6308720e731700ed938c721a018c720b01927e8c721a02069972\
    19720c95937219720d73187319958f7207731a93b2db6308b2a4731b00731c0086029593b17209731d8cb27209731e0\
    0018cb27209731f000173207321";

const DEPOSIT_TEMPLATE_BYTES: &str =
    "d808d601b2a4730000d602db63087201d6037301d604b2a5730200d6057303d606c57201d607b2a5730400d6088cb2\
    db6308a773050002eb027306d1ededed938cb27202730700017203ed93c27204720593860272067308b2db630872047\
    30900ededededed93cbc27207730a93d0e4c672070608720593e4c67207070e72039386028cb27202730b00017208b2\
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

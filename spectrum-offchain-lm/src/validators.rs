use ergo_lib::ergotree_ir::ergo_tree::ErgoTree;
use ergo_lib::ergotree_ir::serialization::SigmaSerializable;

pub fn pool_validator() -> ErgoTree {
    let raw = "19d2041e04000402040204040404040604060408040804040402040004000402040204000400
        19d2041e04000402040204040404040604060408040804040402040004000402040204000400
        040a050004020402050005000402040205000500040205000500d81ad601b2a5730000d602db
        63087201d603db6308a7d604e4c6a70410d605e4c6a70505d606e4c6a70605d607b272027301
        00d608b27203730200d609b27202730300d60ab27203730400d60bb27202730500d60cb27203
        730600d60db27202730700d60eb27203730800d60f8c720a02d610998c720902720fd6118c72
        0802d6129a99a3b27204730900730ad613b27204730b00d6149d72127213d61595919e721272
        13730c9a7214730d7214d616b27204730e00d6177e721605d6189d72057217d619998c720b02
        8c720c02d61a998c720d028c720e02d1ededededed93b27202730f00b27203731000eded93e4
        c672010410720493e4c672010505720593e4c672010605720693c27201c2a7ededed938c7207
        018c720801938c7209018c720a01938c720b018c720c01938c720d018c720e0193b172027311
        959172107312ededec929a997205721172069c7e9995907215721672159a7216731373140572
        189372117205937210f07219939c7210997217a273157e721505f0721a958f72107316ededec
        929a997205721172069c7e9995907215721672159a7216731773180572189372117205937219
        f0721092721a95917215721673199c7219997217a2731a7e721505d801d61be4c672010704ed
        ededed90721b997215731b909972119c7e997216721b0572189a7218720693f0998c72070272
        117d9d9c7e7218067e721a067e720f0605937210731c937219731d";
    let bf = base16::decode(raw.as_bytes()).unwrap();
    ErgoTree::sigma_parse_bytes(&*bf).unwrap()
}

pub fn deposit_validator_temp() -> Vec<u8> {
    let raw = "d808d601b2a4730000d602db63087201d6037301d604b2a5730200d6057303d606c57201d607b2
        a5730400d6088cb2db6308a773050002eb027306d1eded938cb27202730700017203ed93c27204
        720593860272067308b2db63087204730900edededed93e4c67207040e720593e4c67207050e72
        039386028cb27202730a00017208b2db63087207730b009386028cb27202730c00019c7208730d
        b2db63087207730e009386027206730fb2db63087207731000";
    base16::decode(raw.as_bytes()).unwrap()
}

pub fn redeem_validator_temp() -> Vec<u8> {
    let raw = "d801d601b2a5730000eb027301d1ed93c27201730293860273037304b2db63087201730500";
    base16::decode(raw.as_bytes()).unwrap()
}

pub fn bundle_validator() -> ErgoTree {
    let raw = "19ab0315040004000404040404040402040004000500040204020400050204040400040205000404040005fcffffffffffffffff010100d80ed601b2a5730000d602db63087201d603e4c6a7050ed604b2a4730100d605db63087204d6068cb2720573020002d607998cb27202730300027206d608e4c6a7040ed609db6308a7d60a8cb2720973040001d60bb27209730500d60cb27209730600d60d8c720c02d60e8c720b02d1ed938cb27202730700017203959372077308d808d60fb2a5e4e3000400d610b2a5e4e3010400d611db63087210d612b27211730900d613b2e4c672040410730a00d614c672010804d6157e99721395e67214e47214e4c67201070405d616b2db6308720f730b00eded93c2720f7208ededededed93e4c67210040e720893e4c67210050e7203938602720a730cb27211730d00938c7212018c720b019399720e8c72120299720e9c7215720d93b27211730e00720ced938c7216018cb27205730f0001927e8c721602069d9c9c7e9de4c6720405057e721305067e720d067e999d720e720d7215067e720606958f7207731093b2db6308b2a47311007312008602720a73137314";
    let bf = base16::decode(raw.as_bytes()).unwrap();
    ErgoTree::sigma_parse_bytes(&bf).unwrap()
}